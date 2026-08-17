use std::sync::{Arc, Mutex};
use crate::AppState;
use super::TaskQueue;

#[tauri::command]
pub async fn enqueue_validate(
    ids: Vec<String>,
    check_restrictions: Option<bool>,
    check_2fa: Option<bool>,
    check_aging: Option<bool>,
    threads: Option<usize>,
    queue: tauri::State<'_, TaskQueue>,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
) -> Result<String, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let count = ids.len();
    let do_restrictions = check_restrictions.unwrap_or(false);
    let do_2fa = check_2fa.unwrap_or(false);
    let do_aging = check_aging.unwrap_or(false);
    let concurrency = threads.unwrap_or(5).max(1).min(1000);

    dbg_log!("enqueue_validate {} accounts, task_id={}, restrictions={}, 2fa={}, aging={}, threads={}",
        count, task_id, do_restrictions, do_2fa, do_aging, concurrency);

    {
        let mut s = state.lock().unwrap();
        for id in &ids {
            if !s.validating_ids.contains(id) {
                s.validating_ids.push(id.clone());
            }
        }
    }

    // immediately set status to "Проверка..." so frontend sees it right away
    {
        let storage = crate::accounts::commands::get_storage_pub();
        for id in &ids {
            let json_path = storage.json_path(id);
            if let Ok(mut json) = crate::accounts::session::AccountJson::from_file(&json_path) {
                if !json.status.starts_with(&crate::i18n::t("status_checking").trim_end_matches('.').to_string()) {
                    json.status = crate::i18n::t("status_checking");
                    let _ = json.to_file(&json_path);
                }
            }
        }
    }

    crate::accounts::commands::invalidate_accounts_cache();

    let state_arc = Arc::clone(&state);
    let ids_clone = ids.clone();

    queue.enqueue(
        task_id.clone(),
        "validate".to_string(),
        crate::i18n::t_with("validate_task_name", &[("count", &count.to_string())]),
        move || {
            Box::pin(async move {
                dbg_log!("=== VALIDATE TASK START ({} accounts, {} threads) ===", ids_clone.len(), count);
                let storage = crate::accounts::commands::get_storage_pub();
                let proxy_list = Arc::new(crate::proxy::ProxyList::load());
                dbg_log!("validate task: {} proxies available", proxy_list.proxies.len());

                let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
                let mut handles = Vec::new();

                for id in ids_clone.iter() {
                    let id = id.clone();
                    let sem = sem.clone();
                    let state_arc = state_arc.clone();
                    let proxy_list = proxy_list.clone();
                    let storage_base = storage.session_json_dir();
                    let do_r = do_restrictions;
                    let do_2fa_check = do_2fa;

                    handles.push(tokio::spawn(async move {
                        let _permit = sem.acquire().await.unwrap();

                        let id_clone = id.clone();
                        let state_clone = state_arc.clone();
                        // aging check is now instant (id interpolation), no extra timeout needed
                        let task_timeout = 480;
                        let result = tokio::time::timeout(
                            std::time::Duration::from_secs(task_timeout),
                            async {
                                let storage = crate::accounts::storage::AccountStorage::new(&storage_base.parent().unwrap());
                                let session_path = storage.session_path(&id);
                                let json_path = storage.json_path(&id);

                                if !session_path.exists() {
                                    if let Ok(mut s) = state_arc.lock() {
                                        s.validating_ids.retain(|x| x != &id);
                                    }
                                    return;
                                }

                                let session = match crate::accounts::session::TelethonSession::from_file(&session_path) {
                                    Ok(s) => s,
                                    Err(_) => {
                                        if let Ok(mut s) = state_arc.lock() {
                                            s.validating_ids.retain(|x| x != &id);
                                        }
                                        return;
                                    }
                                };

                                let mut json = if json_path.exists() {
                                    crate::accounts::session::AccountJson::from_file(&json_path).unwrap_or_default()
                                } else {
                                    crate::accounts::session::AccountJson {
                                        app_id: 2040,
                                        app_hash: "b18441a1ff607e10a989891a5462e627".to_string(),
                                        ..Default::default()
                                    }
                                };

                                let account_proxy = json.proxy.as_ref()
                                    .and_then(|ps| crate::proxy::ProxyConfig::from_string(ps).ok());
                                let random_proxy = proxy_list.get_random().cloned();
                                let proxy_ref = account_proxy.as_ref().or(random_proxy.as_ref());

                                let original_status = json.status.clone();

                                let delays: [u64; 5] = [0, 300, 500, 1500, 1500];
                                let mut final_result = None;
                                for attempt in 1..=5 {
                                    if attempt > 1 {
                                        dbg_log!("validate {}: attempt {}/5 (prev was unreachable)", id, attempt);
                                        tokio::time::sleep(std::time::Duration::from_millis(delays[attempt - 1])).await;
                                        // only surface the attempt counter during real retries —
                                        // the first pass keeps the uniform "Проверка..." label so
                                        // queued and in-progress accounts look the same at start.
                                        json.status = format!("{}", crate::i18n::t_with("validate_checking_attempt", &[("attempt", &attempt.to_string())]));
                                    } else {
                                        json.status = crate::i18n::t("status_checking");
                                    }
                                    let _ = json.to_file(&json_path);

                                    let (vr, cli) = crate::checker::validate::validate_account(&session, &json, proxy_ref).await;

                                    if vr.valid || !vr.unreachable {
                                        final_result = Some((vr, cli));
                                        break;
                                    }

                                    if attempt == 5 {
                                        final_result = Some((vr, None));
                                    }
                                }

                                let (vr, mut reusable_client) = final_result.unwrap();
                                json.validated = true;
                                json.valid = vr.valid;
                                if let Some(ref phone) = vr.phone { json.phone = phone.clone(); }
                                if let Some(ref name) = vr.first_name { json.first_name = name.clone(); }
                                if let Some(ref last) = vr.last_name { json.last_name = last.clone(); }
                                if let Some(ref uname) = vr.username { json.username = uname.clone(); }
                                if let Some(uid) = vr.user_id { json.user_id = uid; }
                                if let Some(prem) = vr.premium {
                                    json.is_premium = prem;
                                    if prem && json.role != crate::i18n::t("role_checker") {
                                        json.role = crate::i18n::t("role_premium");
                                    }
                                }

                                let prev_status = original_status.clone();
                                json.status = if vr.valid {
                                    if do_r {
                                        json.status = crate::i18n::t("validate_restrictions");
                                        let _ = json.to_file(&json_path);

                                        let restriction_status = if let Some(ref mut client) = reusable_client {
                                            match crate::checker::checks::check_spambot(client).await {
                                                Ok(s) => s,
                                                Err(_) => crate::i18n::t("status_clean"),
                                            }
                                        } else {
                                            crate::i18n::t("status_clean")
                                        };
                                        restriction_status
                                    } else {
                                        let dominated = [crate::i18n::t("status_perm_spam"), crate::i18n::t("status_frozen")];
                                        let is_temp = prev_status.starts_with(&crate::i18n::t("status_geo_spam"));
                                        if dominated.iter().any(|s| prev_status == *s) || is_temp {
                                            prev_status
                                        } else {
                                            crate::i18n::t("status_clean")
                                        }
                                    }
                                } else if vr.unreachable {
                                    crate::i18n::t("status_unchecked")
                                } else {
                                    crate::i18n::t("status_invalid")
                                };

                                if do_2fa_check && vr.valid {
                                    if let Some(ref mut client) = reusable_client {
                                        if let Ok((has_2fa, hint)) = crate::checker::checks::check_2fa_with_hint(client).await {
                                            if has_2fa && json.two_fa.is_empty() {
                                                if hint.is_empty() {
                                                    json.two_fa = crate::i18n::t("two_fa_unknown");
                                                } else {
                                                    json.two_fa = crate::i18n::t_with("two_fa_unknown_hint", &[("hint", &hint)]);
                                                }
                                            }
                                        }
                                    }
                                }

                                // aging — always estimate registration date from user_id
                                if vr.valid {
                                    if let Some(uid) = vr.user_id {
                                        let ts = crate::accounts::aging::estimate_registration_ts(uid);
                                        if ts > 0 {
                                            json.register_time = ts;
                                        }
                                    }
                                }

                                let _ = json.to_file(&json_path);

                                if let Ok(mut s) = state_arc.lock() {
                                    s.validating_ids.retain(|x| x != &id);
                                }
                                crate::accounts::commands::invalidate_accounts_cache();
                            }
                        ).await;

                        if result.is_err() {
                            let storage = crate::accounts::storage::AccountStorage::new(&storage_base.parent().unwrap());
                            let json_path = storage.json_path(&id_clone);
                            if let Ok(mut json) = crate::accounts::session::AccountJson::from_file(&json_path) {
                                json.status = crate::i18n::t("status_unchecked");
                                let _ = json.to_file(&json_path);
                            }
                            if let Ok(mut s) = state_clone.lock() {
                                s.validating_ids.retain(|x| x != &id_clone);
                            }
                        }
                    }));
                }

                for h in handles {
                    let _ = h.await;
                }

                dbg_log!("=== VALIDATE TASK DONE ===");
                Ok(())
            })
        },
    );

    Ok(tid)
}
