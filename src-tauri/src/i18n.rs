use std::sync::OnceLock;
use std::sync::Mutex;
use std::collections::HashMap;

static LOCALE: OnceLock<Mutex<String>> = OnceLock::new();

fn get_locale_mutex() -> &'static Mutex<String> {
    LOCALE.get_or_init(|| Mutex::new("ru".to_string()))
}

pub fn get_locale() -> String {
    get_locale_mutex().lock().unwrap().clone()
}

pub fn set_locale_value(locale: &str) {
    let mut l = get_locale_mutex().lock().unwrap();
    *l = locale.to_string();
}

#[tauri::command]
pub fn set_locale(locale: String) {
    set_locale_value(&locale);
    // Invalidate accounts cache so status/aging strings are re-generated in new locale
    crate::accounts::commands::invalidate_accounts_cache();
}

/// Translate a key to the current locale.
/// Usage: `t("done")`, `t("error")`, etc.
/// Falls back to the key itself if not found.
pub fn t(key: &str) -> String {
    let locale = get_locale();
    let dict = if locale == "en" { &*EN } else { &*RU };
    dict.get(key).map(|s| s.to_string()).unwrap_or_else(|| key.to_string())
}

/// Translate with a single parameter substitution.
/// `t_with("invited_user", &[("username", "@john")])` 
pub fn t_with(key: &str, params: &[(&str, &str)]) -> String {
    let mut result = t(key);
    for (k, v) in params {
        result = result.replace(&format!("{{{}}}", k), v);
    }
    result
}

lazy_static::lazy_static! {
    static ref RU: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        // Common
        m.insert("done", "Завершено");
        m.insert("error", "ОШИБКА");
        m.insert("stopped_by_user", "Остановлено пользователем");
        m.insert("db_open_error", "открыть БД: {error}");
        m.insert("db_create_tables_error", "создать таблицы: {error}");
        
        // Parser
        m.insert("parser_collected", "собрано: {count}");
        m.insert("parser_group_done", "группа обработана: {name}");
        m.insert("parser_joining", "вступаю в {name}...");
        m.insert("parser_leaving", "выхожу из {name}...");
        m.insert("parser_flood_wait", "FLOOD_WAIT {seconds} сек, жду...");
        m.insert("parser_flood_skip", "FLOOD_WAIT {seconds} сек > лимит, пропуск");
        m.insert("parser_no_accounts", "не выбраны аккаунты");
        m.insert("parser_no_targets", "не указаны цели для парсинга");
        m.insert("parser_task_name", "Парсер: {groups} групп, {accounts} акк.");
        m.insert("parser_start", "Старт: {groups} групп, {accounts} аккаунтов, {threads} потоков");
        m.insert("parser_db_path", "БД: {path}");
        m.insert("parser_thread_start", "начинаю: {target}");
        m.insert("parser_thread_done", "готово: +{count} пользователей");
        m.insert("parser_export_txt", "Экспорт TXT: {path}...");
        m.insert("parser_txt_exported", "TXT экспортирован: {path}");
        m.insert("parser_total", "Итого: {done} групп обработано, {total} уникальных пользователей");
        m.insert("parser_no_filter", "не выбран ни один фильтр участников");
        m.insert("parser_prefix", "[акк {acc}/группа {group}/{total}]");
        m.insert("parser_target", "цель: id={id} {hint}");
        m.insert("parser_target_private", "(приватная)");
        m.insert("parser_broadcast_not_group", "это broadcast-канал, а не группа");
        m.insert("parser_no_admin_rights", "нет прав администратора");
        m.insert("parser_access_check", "проверка доступа: {error}");
        m.insert("parser_admins_loaded", "Загружено админов: {count}");
        m.insert("parser_admins_error", "Не удалось получить список админов: {error}");
        m.insert("parser_method", "Метод: {method}");
        m.insert("parser_method_alphabet", "поиск по алфавиту ({count} символов)");
        m.insert("parser_method_pagination", "прямая пагинация (пустой query)");
        m.insert("parser_parse_error", "Ошибка разбора (char='{char}' offset={offset}): {error}");
        m.insert("parser_channel_private_wait", "ChannelPrivate — жду 45 сек...");
        m.insert("parser_flood_wait_short", "FLOOD_WAIT {seconds} сек — жду...");
        m.insert("parser_members_hidden", "список участников скрыт");
        m.insert("parser_error", "Ошибка: {error}");
        m.insert("parser_char_progress", "[{done}/{total}] char='{char}': +{added} (собрано: {collected}, просмотрено: {viewed})");
        m.insert("parser_collected_progress", "Собрано: {count}");
        m.insert("parser_msg_mode_days", "Режим сообщений: собираю авторов за последние {days} дней");
        m.insert("parser_msg_mode_all", "Режим сообщений: собираю авторов из всей истории");
        m.insert("parser_flood_over_limit", "FLOOD_WAIT {seconds} > лимит — остановка");
        m.insert("parser_get_history_error", "Ошибка getHistory: {error}");
        m.insert("parser_msg_history_end", "Конец истории сообщений");
        m.insert("parser_days_limit", "Достигнут лимит {days} дней — остановка");
        m.insert("parser_msg_progress", "Сообщений: {messages}, собрано уникальных: {collected}");
        m.insert("parser_msg_total", "Итого сообщений: {messages}, уникальных авторов: {collected}");
        m.insert("parser_comment_mode_days", "Режим комментариев: глубина {days} дней");
        m.insert("parser_comment_mode_all", "Режим комментариев: вся история");
        m.insert("parser_posts_history_end", "Конец истории постов");
        m.insert("parser_posts_progress", "Постов: {scanned}, с комментами: {with_comments}, собрано: {collected}");
        m.insert("parser_days_limit_short", "Достигнут лимит {days} дней");
        m.insert("parser_posts_total", "Итого постов: {scanned}, с комментариями: {with_comments}, собрано: {collected}");
        
        // Inviter
        m.insert("inviter_invited", "приглашён: {username}");
        m.insert("inviter_added", "добавлен: {username}");
        m.insert("inviter_peer_flood", "PEER_FLOOD: аккаунт остановлен");
        m.insert("inviter_flood_wait", "FLOOD_WAIT {seconds} сек");
        m.insert("inviter_autostop", "автостоп: достигнут лимит ошибок");
        
        // Boost
        m.insert("boost_done", "выполнено: {detail}");
        m.insert("boost_bot_started", "бот запущен: {name}");
        m.insert("boost_subscribed", "подписка оформлена: {channel}");
        m.insert("boost_reacted", "реакция поставлена");
        m.insert("boost_viewed", "просмотрено");
        m.insert("boost_task_name", "Накрутка: {count} акк.");
        m.insert("boost_no_bot_links", "не указаны ссылки на ботов");
        m.insert("boost_bot_error", "{prefix} ошибка бота {link}: {error}");
        m.insert("boost_empty_bot_link", "пустая ссылка на бота");
        m.insert("boost_bot_activated", "{prefix} бот @{username} активирован, выполнено");
        m.insert("boost_bot_activated_ref", "{prefix} бот @{username} активирован с реф='{payload}', выполнено");
        m.insert("boost_bot_blocked", "{prefix} бот @{username} заблокирован и удалён");
        m.insert("boost_no_posts_to_view", "нет постов для просмотра");
        m.insert("boost_views_done", "{prefix} просмотр {count} постов в '{channel}' выполнено");
        m.insert("boost_join_failed", "{prefix} не удалось вступить в канал: {error}");
        m.insert("boost_no_reaction_links", "нет ссылок для реакций");
        m.insert("boost_no_posts_for", "{prefix} {label}: нет постов");
        m.insert("boost_joined", "{prefix} вступил в {label}");
        m.insert("boost_join_failed_label", "{prefix} не удалось вступить в {label}: {error}");
        m.insert("boost_resolve_error", "{prefix} ошибка resolve {link}: {error}");
        m.insert("boost_no_posts_for_reactions", "нет доступных постов для реакций");
        m.insert("boost_reactions_db", "{prefix} БД реакций: {path}");
        m.insert("boost_no_emoji_skip", "{prefix} не выбрано эмодзи, пропуск");
        m.insert("boost_reaction_progress", "{prefix} {emoji} {label}/{msg_id} реакция {idx} [{ok}/{total}]");
        m.insert("boost_reaction_limit", "{prefix} лимит реакций на {label}/{msg_id}");
        m.insert("boost_reaction_error", "{prefix} ошибка {label}/{msg_id}: {error}");
        m.insert("boost_reactions_done", "{prefix} реакции завершены: ок={ok}, ошибок={errs}");
        m.insert("boost_no_reactions_sent", "ни одна реакция не отправлена");
        m.insert("boost_invalid_post_link", "невалидная ссылка на пост");
        m.insert("boost_empty_channel_link", "пустая ссылка на канал");
        m.insert("boost_join_link_done", "{prefix} вступление по ссылке {link} выполнено");
        m.insert("boost_archive_error", "{prefix} ошибка архивации: {error}");
        m.insert("boost_archived", "{prefix} перемещено в архив");
        m.insert("boost_empty_folder_links", "пустой список ссылок на папки");
        m.insert("boost_not_addlist", "{prefix} ошибка: '{link}' не выглядит как t.me/addlist/");
        m.insert("boost_check_invite_error", "{prefix} ошибка checkChatlistInvite '{slug}': {error}");
        m.insert("boost_parse_invite_error", "{prefix} ошибка разбора invite '{slug}': {error}");
        m.insert("boost_folder_imported", "{prefix} папка '{name}' импортирована, выполнено");
        m.insert("boost_join_invite_error", "{prefix} ошибка joinChatlistInvite '{slug}': {error}");
        m.insert("boost_no_folders_imported", "ни одна из {count} папок не импортирована");
        m.insert("boost_parse_link_error", "не удалось распарсить ссылку: {link}");
        m.insert("boost_sub_username_failed", "не удалось подписаться через username");
        m.insert("boost_addlist_unsupported", "addlist-ссылка не поддерживается в этом режиме");
        m.insert("boost_unknown_link_type", "неизвестный тип ссылки: {kind}");
        m.insert("boost_left_channel", "{prefix} вышли из канала");
        m.insert("boost_leave_error", "{prefix} ошибка выхода: {error}");
        m.insert("boost_reactions_db_open_error", "открыть БД реакций: {error}");
        
        // Cloner
        m.insert("cloner_copied", "скопировано: пост #{id}");
        m.insert("cloner_skipped", "пропущено: пост #{id}");
        m.insert("cloner_channel_created", "канал создан: {name}");
        
        // Auto-Reply
        m.insert("auto_reply_sent", "ответ отправлен: {username}");
        m.insert("auto_reply_skipped", "пропущено: {reason}");
        m.insert("auto_reply_voice_sent", "голосовое отправлено: {username}");
        m.insert("auto_reply_ban_word", "бан-слово: пропуск");
        m.insert("auto_reply_no_accounts", "не выбраны аккаунты");
        m.insert("auto_reply_task_name", "Автоответчик: {count} акк.");
        m.insert("auto_reply_audio_convert_error", "Ошибка конвертации аудио: {error}");
        m.insert("auto_reply_voice_read_error", "Ошибка чтения голосового: {error}");
        m.insert("auto_reply_image_read_error", "Ошибка чтения изображения: {error}");
        m.insert("auto_reply_video_read_error", "Ошибка чтения видео: {error}");
        m.insert("auto_reply_db_create_error", "Ошибка создания БД: {error}");
        m.insert("auto_reply_db_open_error", "открыть БД: {error}");
        m.insert("auto_reply_db_tables_error", "создать таблицы: {error}");
        m.insert("auto_reply_connected", "подключён (user_id={user_id})");
        m.insert("auto_reply_uploading_image", "загрузка изображения...");
        m.insert("auto_reply_image_uploaded", "изображение загружено");
        m.insert("auto_reply_uploading_video", "загрузка видео...");
        m.insert("auto_reply_video_uploaded", "видео загружено");
        m.insert("auto_reply_listening", "слушаю входящие сообщения...");
        m.insert("auto_reply_stopped", "остановлено");
        m.insert("auto_reply_limit_reached", "достигнут лимит {limit} ответов");
        m.insert("auto_reply_get_diff_error", "getDifference ошибка: {error}");
        m.insert("auto_reply_parse_diff_error", "parse diff ошибка: {error}");
        m.insert("auto_reply_no_access_hash", "пропущено: не удалось получить access_hash для user_id={user_id}");
        m.insert("auto_reply_not_in_list", "пропущено (не в списке): user_id={user_id}");
        m.insert("auto_reply_ban_word_skip", "пропущено (бан-слово): user_id={user_id}");
        m.insert("auto_reply_voice_not_loaded", "голосовой файл не загружен");
        m.insert("auto_reply_invalid_forward_id", "неверный ID сообщения для пересылки");
        m.insert("auto_reply_voice_label", "голосовое отправлено");
        m.insert("auto_reply_forward_label", "переслано");
        m.insert("auto_reply_reply_label", "ответ отправлен");
        m.insert("auto_reply_send_error", "ошибка отправки user_id={user_id}: {error}");
        
        // First Comment
        m.insert("first_comment_sent", "комментарий отправлен: {channel}");
        m.insert("first_comment_monitoring", "мониторинг каналов...");
        m.insert("first_comment_new_post", "новый пост в {channel}");
        
        // Reporter
        m.insert("reporter_sent", "репорт отправлен: {target}");
        m.insert("reporter_db_path", "БД: {path}");
        m.insert("reporter_db_create_error", "Не удалось создать БД: {error}");
        m.insert("reporter_task_name", "Репортер: {count} акк.");
        m.insert("reporter_username_invalid", "юзернейм @{target} невалиден");
        m.insert("reporter_username_not_exists", "юзернейм @{target} не существует");
        m.insert("reporter_target_info", "цель: @{target} (id={id}, {kind})");
        m.insert("reporter_target_channel", "канал");
        m.insert("reporter_target_user", "юзер");
        m.insert("reporter_report_sent", "репорт отправлен [{idx}/{total}] причина={reason}");
        m.insert("reporter_error", "ошибка: {error}");
        m.insert("reporter_invalid_link", "невалидная ссылка на пост/канал");
        m.insert("reporter_invalid_channel_id", "невалидный ID канала");
        m.insert("reporter_no_posts", "{target}: нет постов для репорта");
        m.insert("reporter_target_posts", "цель: {target} ({count} постов)");
        m.insert("reporter_channel_report_sent", "репорт {target}/{msg_id} [{idx}/{total}] причина={reason}");
        m.insert("reporter_error_sub", "ошибка (sub): {error}");
        m.insert("reporter_error_comment", "ошибка (comment): {error}");
        m.insert("reporter_bot_resolved", "бот resolved: id={id}");
        m.insert("reporter_bot_report_sent", "репорт отправлен [{idx}/{total}] запрос='{query}'");
        m.insert("reporter_bot_blocked", "бот заблокирован, история удалена");
        m.insert("reporter_photo_target", "фото-репорт: @{target} (id={id})");
        m.insert("reporter_photo_fetch_error", "ошибка получения фото: {error}");
        m.insert("reporter_no_photos", "@{target}: нет фото профиля");
        m.insert("reporter_photos_count", "фото профиля: {count}");
        m.insert("reporter_photo_report_sent", "фото-репорт [{idx}/{total}] причина={reason}");
        m.insert("reporter_photo_report_error", "ошибка фото-репорт: {error}");
        m.insert("reporter_db_open_error", "открыть БД репортера: {error}");
        m.insert("reporter_db_tables_error", "создать таблицы: {error}");
        
        // Masslooking
        m.insert("masslooking_viewed", "просмотрено: {username}");
        m.insert("masslooking_reacted", "реакция: {username}");
        m.insert("masslooking_replied", "ответ отправлен: {username}");
        
        // Mailing
        m.insert("mailing_sent", "отправлено: {target}");
        m.insert("mailing_ban", "бан на аккаунте");
        m.insert("mailing_spamblock", "спамблок");
        m.insert("mailing_skip_spamblock", "пропуск: спамблок ({status})");
        m.insert("mailing_peer_flood_retry", "PEER_FLOOD — ещё одна попытка...");
        m.insert("mailing_peer_flood_stop", "PEER_FLOOD дважды — останавливаем поток");
        m.insert("mailing_no_accounts", "не выбраны аккаунты");
        m.insert("mailing_task_name", "Рассылка: {count} акк.");
        m.insert("mailing_waiting", "ожидание до {time} ({ms} мс)...");
        m.insert("mailing_repost_group_created", "auto-repost: группа создана");
        m.insert("mailing_repost_msg_sent", "auto-repost: сообщение отправлено в temp-группу");
        m.insert("mailing_repost_no_msg_id", "auto-repost: не удалось получить msg_id");
        m.insert("mailing_repost_send_error", "auto-repost: ошибка отправки: {error}");
        m.insert("mailing_repost_no_chat_id", "auto-repost: не удалось распарсить chat_id");
        m.insert("mailing_repost_create_error", "auto-repost: ошибка создания группы: {error}");
        m.insert("mailing_sent_to_chat", "отправлено в {target}");
        m.insert("mailing_send_error_chat", "ошибка отправки в {target}");
        m.insert("mailing_no_posts_in", "нет постов в {target}");
        m.insert("mailing_comment_sent", "комментарий в {target} к посту {msg_id}");
        m.insert("mailing_comment_error", "ошибка комментария {target}: {error}");
        m.insert("mailing_no_phones", "нет номеров для рассылки");
        m.insert("mailing_phone_resolve_error", "не удалось резолвить номер: {phone}");
        m.insert("mailing_no_story_link", "не указана ссылка на историю");
        m.insert("mailing_story_parse_error", "не удалось распарсить ссылку на историю: {link}");
        m.insert("mailing_story_forwarded", "история переслана -> @{username}");
        m.insert("mailing_story_forward_error", "ошибка пересылки @{username}: {error}");
        m.insert("mailing_story_owner_resolve_error", "не удалось резолвить владельца истории: {username}");
        m.insert("mailing_repost_group_deleted", "auto-repost: temp-группа удалена");
        m.insert("mailing_total_sent", "итого отправлено: {count}");
        m.insert("mailing_sent_user", "отправлено user_id={user_id}");
        m.insert("mailing_error_user", "ошибка user_id={user_id}: {error}");
        
        // Stories
        m.insert("stories_published", "опубликовано");
        
        // Warmer
        m.insert("warmer_action_done", "действие выполнено: {action}");
        m.insert("warmer_no_accounts", "не выбраны аккаунты");
        m.insert("warmer_task_name", "Прогрев: {count} акк.");
        m.insert("warmer_starting", "Запуск прогрева на {count} аккаунтах");
        m.insert("warmer_acc_error", "[акк {idx}/{total}] ОШИБКА: {error}");
        m.insert("warmer_no_uid_warning", "ВНИМАНИЕ: не удалось определить user_id, fake_chat будет недоступен");
        m.insert("warmer_spamblock_json", "спамблок из JSON: {status}");
        m.insert("warmer_spamblock_detected", "СПАМБЛОК: {status}");
        m.insert("warmer_no_spamblock", "спамблок не обнаружен");
        m.insert("warmer_spamblock_peer_flood", "СПАМБЛОК (PEER_FLOOD)");
        m.insert("warmer_temp_tag_set", "установлен временный тег @{username}");
        m.insert("warmer_no_actions", "нет включённых действий");
        m.insert("warmer_duration", "работаю {minutes} мин");
        m.insert("warmer_until_stop", "работаю до остановки");
        m.insert("warmer_saved", "избранное: \"{text}\"");
        m.insert("warmer_saved_no_msgid", "не удалось извлечь msg_id из ответа saved");
        m.insert("warmer_del_msg_error", "ошибка удаления msg {id}: {error}");
        m.insert("warmer_saved_send_error", "ошибка отправки в избранное: {error}");
        m.insert("warmer_search", "поиск: {word}");
        m.insert("warmer_search_error", "ошибка поиска: {error}");
        m.insert("warmer_read_channel", "читаю канал: поиск '{word}'");
        m.insert("warmer_contacts_search_error", "ошибка contacts.search: {error}");
        m.insert("warmer_reaction", "реакция {emoji} на пост {id}");
        m.insert("warmer_reactions_disabled", "реакции выключены для канала {id}");
        m.insert("warmer_read_dialogs", "читаю диалоги");
        m.insert("warmer_read_dialogs_error", "ошибка чтения диалогов: {error}");
        m.insert("warmer_view_stories", "просматриваю истории");
        m.insert("warmer_stories_read_error", "ошибка чтения историй: {error}");
        m.insert("warmer_stories_viewed", "просмотрено {count} историй канала {id}");
        m.insert("warmer_subscribing", "подписка на канал из поиска");
        m.insert("warmer_search_result", "contacts.search '{word}': len={len}");
        m.insert("warmer_search_empty_fallback", "contacts.search пуст, fallback resolve @{name}");
        m.insert("warmer_resolve_error", "resolve @{name} ошибка: {error}");
        m.insert("warmer_parse_resolve_error", "parse_resolved_peer ошибка: {error}");
        m.insert("warmer_unsubscribe_error", "ошибка отписки от старого канала {id}: {error}");
        m.insert("warmer_subscribed", "подписался на канал id={id}");
        m.insert("warmer_join_error", "ошибка joinChannel {id}: {error}");
        m.insert("warmer_no_channel_found", "не удалось найти канал ни через поиск ни через resolve");
        m.insert("warmer_group_members", "смотрю участников группы");
        m.insert("warmer_group_members_count", "группа id={id}: {count} участников");
        m.insert("warmer_photo_error", "ошибка загрузки фото user {id}: {error}");
        m.insert("warmer_avatars_viewed", "просмотрено {count} аватарок");
        m.insert("warmer_del_contact_error", "ошибка удаления контакта {id}: {error}");
        m.insert("warmer_contact_added_group", "добавлен контакт из группы id={id}");
        m.insert("warmer_resolve_username_error", "resolve username '{username}' ошибка: {error}");
        m.insert("warmer_skip_fake_chat", "пропуск fake_chat: не удалось зарезолвить пир (user={id}, username='{username}')");
        m.insert("warmer_llm_error", "LLM ошибка: {error}, fallback на random");
        m.insert("warmer_fake_chat_msg", "фейк-чат -> user {id}: \"{text}\"");
        m.insert("warmer_send_error", "ошибка отправки: {error}");
        m.insert("warmer_spamblock_on_send", "спамблок при отправке: {error}");
        m.insert("warmer_too_many_send_errors", "слишком много ошибок отправки, возможен спамблок");
        m.insert("warmer_fake_chat_no_peers", "фейк-чат (нет пиров): избранное \"{text}\"");
        m.insert("warmer_read_telegram", "читаю переписку с Telegram");
        m.insert("warmer_read_telegram_error", "ошибка чтения 777000: {error}");
        m.insert("warmer_add_contact", "добавляю контакт из поиска");
        m.insert("warmer_contact_added", "добавлен контакт id={id}");
        m.insert("warmer_contact_add_error", "ошибка добавления контакта {id}: {error}");
        m.insert("warmer_del_old_contact_error", "ошибка удаления старого контакта {id}: {error}");
        m.insert("warmer_search_no_users", "поиск не вернул пользователей");
        m.insert("warmer_rest", "отдых {seconds} сек...");
        m.insert("warmer_cleanup_start", "начало очистки: каналов={channels}, saved_msgs={msgs}, fake_chats={chats}, контактов={contacts}");
        m.insert("warmer_cleanup_unsub", "очистка: отписка от {count} каналов");
        m.insert("warmer_cleanup_unsub_error", "ошибка отписки от {id}: {error}");
        m.insert("warmer_cleanup_saved", "очистка: удаление ~{count} сообщений из избранного");
        m.insert("warmer_cleanup_saved_found", "найдено {count} сообщений в избранном, удаляю");
        m.insert("warmer_cleanup_saved_no_ids", "не удалось получить ID сообщений из избранного");
        m.insert("warmer_cleanup_saved_history_error", "ошибка получения истории избранного: {error}");
        m.insert("warmer_cleanup_chats", "очистка: удаление переписок с {count} пирами");
        m.insert("warmer_cleanup_chat_error", "ошибка удаления истории с {id}: {error}");
        m.insert("warmer_cleanup_contacts", "очистка: удаление {valid} контактов (из {total} tracked)");
        m.insert("warmer_cleanup_contacts_error", "ошибка удаления контактов: {error}");
        m.insert("warmer_cleanup_contacts_skip", "очистка: {count} контактов с access_hash=0, пропуск");
        m.insert("warmer_cleanup_tag_error", "ошибка удаления временного тега: {error}");
        m.insert("warmer_cleanup_tag_done", "временный тег удалён");
        m.insert("warmer_session_done", "сессия завершена ({count} действий)");
        
        // Channel Creator
        m.insert("channel_created", "создан: {name}");
        
        // Bot Creator
        m.insert("bot_created", "бот создан: {username}");
        m.insert("bot_token", "токен: {token}");
        
        // Global Search
        m.insert("search_found", "новых: {count}");
        
        // Username Checker
        m.insert("username_free", "свободен: {username}");
        m.insert("username_taken", "занят: {username}");
        m.insert("username_fragment", "fragment: {username}");
        
        // Converter
        m.insert("converter_ok", "конвертировано: {path}");
        m.insert("converter_err", "ошибка: {path}");
        m.insert("converter_unknown_format", "неизвестный формат: {format}");
        m.insert("converter_telethon_read", "[telethon] прочитан: {path}");
        m.insert("converter_telethon_error", "[telethon] ошибка {path}: {error}");
        m.insert("converter_pyrogram_read", "[pyrogram] прочитан: {path}");
        m.insert("converter_pyrogram_error", "[pyrogram] ошибка {path}: {error}");
        m.insert("converter_tdata_not_found", "[tdata] не найдено tdata в {path}");
        m.insert("converter_tdata_read", "[tdata] прочитано {count} акк. из {path}");
        m.insert("converter_tdata_error", "[tdata] ошибка {path}: {error}");
        m.insert("converter_tdatazip_unpack_error", "[tdata_zip] ошибка распаковки {path}: {error}");
        m.insert("converter_tdatazip_read", "[tdata_zip] прочитано {count} акк. из {path}");
        m.insert("converter_tdatazip_error", "[tdata_zip] ошибка tdata {path}: {error}");
        m.insert("converter_authkey_read_error", "[authkey] ошибка чтения {path}: {error}");
        m.insert("converter_authkey_invalid_hex", "[authkey] {path} строка {line}: невалидный hex");
        m.insert("converter_authkey_no_dc", "[authkey] {path} строка {line}: не удалось определить DC");
        m.insert("converter_authkey_read", "[authkey] прочитано {count} ключей из {path}");
        m.insert("converter_no_userid", "user_id=0 — невозможно создать TData без user_id. Сначала добавьте в панель и провалидируйте.");
        m.insert("converter_tdatazip_unsupported_out", "формат TdataZip не поддерживается как выходной");
        m.insert("converter_open_file_error", "не удалось открыть файл: {error}");
        m.insert("converter_write_error", "ошибка записи: {error}");
        m.insert("converter_tdatazip_unsupported_target", "Целевой формат TdataZip не поддерживается");
        m.insert("converter_reading_paths", "Чтение {count} путей из формата {format}...");
        m.insert("converter_accounts_found", "Найдено {count} аккаунтов для конвертации");
        m.insert("converter_outdir_error", "Не удалось создать выходную папку: {error}");
        m.insert("converter_added_to_panel", "    добавлен в панель id={id}");
        m.insert("converter_not_added_to_panel", "    в панель не добавлен: {error}");
        m.insert("converter_summary", "Готово. Успешно: {ok} | Ошибок: {err}");
        m.insert("converter_ok_short", "ОК");
        m.insert("llm_rephrase_prompt", "Перефразируй это сообщение немного другими словами, сохрани смысл и длину. Без кавычек, без эмодзи. От себя ничего не добавляй.");
        m.insert("converter_unpack_over_limit", "распаковано >1 ГБ, прервано");
        m.insert("converter_read_error", "Ошибка чтения {path}: {error}");
        m.insert("converter_invalid_authkey_len", "Строка {line}: неверная длина auth_key ({bytes} байт)");
        m.insert("converter_invalid_hex_line", "Строка {line}: невалидный hex");
        m.insert("converter_all_complete", "Все {count} строк уже имеют authkey:dc_id, запись без изменений.");
        m.insert("converter_detecting_dc", "Определение DC для {count} ключей ({threads} потоков)...");
        m.insert("converter_line_dc", "  строка {line}: DC={dc}");
        m.insert("converter_line_no_dc", "  строка {line}: DC не определён, пропущена");
        m.insert("converter_lines_written", "Записано {count} строк в {path}");
        m.insert("converter_write_error_short", "Ошибка записи: {error}");
        m.insert("converter_task_name", "Конвертер");

        // Account Actions
        m.insert("actions_working_on_accounts", "Работа с аккаунтами: {count} акк.");
        m.insert("actions_not_enough_usernames", "Недостаточно юзернеймов: {usable} в файле, {total} аккаунтов");
        m.insert("actions_db_path", "БД: {path}");
        m.insert("actions_account_dead", "ОШИБКА: Аккаунт умер (сессия недействительна)");
        m.insert("actions_delete_account", "удаление аккаунта...");
        m.insert("actions_account_deleted", "аккаунт удалён");
        m.insert("actions_delete_username", "удаление юзернейма...");
        m.insert("actions_username_deleted", "юзернейм удалён");
        m.insert("actions_username_not_set", "юзернейм не установлен");
        m.insert("actions_changing_username", "смена юзернейма на @{username}...");
        m.insert("actions_username_changed", "юзернейм изменён: @{username}");
        m.insert("actions_username_unchanged", "юзернейм не изменён (уже @{username})");
        m.insert("actions_username_unavailable", "@{username} недоступен, генерирую новый...");
        m.insert("actions_username_unavail_err", "юзернейм @{username} недоступен: {error}");
        m.insert("actions_username_error", "юзернейм ошибка: {error}");
        m.insert("actions_delete_avatars", "удаление аватарок...");
        m.insert("actions_avatars_deleted", "удалено {count} аватарок");
        m.insert("actions_no_avatars", "аватарок нет");
        m.insert("actions_photo_error", "фото ошибка: {error}");
        m.insert("actions_photo_get_error", "ошибка получения фото: {error}");
        m.insert("actions_photo_dl_error", "авто-фото ошибка скачивания: {error}");
        m.insert("actions_photo_ul_error", "авто-фото ошибка загрузки: {error}");
        m.insert("actions_delete_stories", "удаление всех историй...");
        m.insert("actions_stories_error", "ошибка удаления историй: {error}");
        m.insert("actions_stories_get_error", "ошибка получения историй: {error}");
        m.insert("actions_stories_deleted", "удалено {count} историй");
        m.insert("actions_no_stories", "историй нет");
        m.insert("actions_set_photo", "установка фото...");
        m.insert("actions_photo_set", "фото установлено");
        m.insert("actions_gen_photo", "генерация авто-фото...");
        m.insert("actions_autophoto_set", "авто-фото установлено");
        m.insert("actions_set_emoji_avatar", "установка эмодзи аватара...");
        m.insert("actions_emoji_avatar_set", "эмодзи аватар установлен");
        m.insert("actions_emoji_avatar_error", "эмодзи аватар ошибка: {error}");
        m.insert("actions_emoji_get_error", "ошибка получения эмодзи: {error}");
        m.insert("actions_emoji_req_error", "ошибка запроса эмодзи: {error}");
        m.insert("actions_emoji_no_valid", "не удалось подобрать валидный эмодзи за 3 попытки");
        m.insert("actions_emoji_list_empty", "список эмодзи пуст");
        m.insert("actions_changing_name", "смена имени: {first} {last}...");
        m.insert("actions_name_set", "имя: {first} {last}");
        m.insert("actions_set_bio", "смена биографии...");
        m.insert("actions_bio_updated", "биография обновлена");
        m.insert("actions_delete_bio", "удаление биографии...");
        m.insert("actions_bio_deleted", "биография удалена");
        m.insert("actions_set_birthday", "установка ДР: {d}.{m}.{y}...");
        m.insert("actions_birthday_set", "ДР установлен");
        m.insert("actions_delete_contacts", "удаление контактов...");
        m.insert("actions_contacts_deleted", "удалено {count} контактов");
        m.insert("actions_no_contacts", "контактов нет");
        m.insert("actions_contacts_del_error", "ошибка удаления контактов: {error}");
        m.insert("actions_contacts_get_error", "ошибка получения контактов: {error}");
        m.insert("actions_contacts_req_error", "ошибка запроса контактов: {error}");
        m.insert("actions_delete_dialogs", "удаление всех диалогов...");
        m.insert("actions_dialogs_deleted", "удалено {count} диалогов");
        m.insert("actions_dialogs_get_error", "ошибка получения диалогов: {error}");
        m.insert("actions_dialogs_parse_error", "ошибка парсинга диалогов: {error}");
        m.insert("actions_delete_bot_dialogs", "удаление диалогов с ботами...");
        m.insert("actions_bots_deleted", "удалено и заблокировано {count} ботов");
        m.insert("actions_read_dialogs", "чтение всех переписок...");
        m.insert("actions_dialogs_read", "прочитано {count} диалогов");
        m.insert("actions_read_error", "ошибка чтения переписок: {error}");
        m.insert("actions_delete_folders", "удаление папок...");
        m.insert("actions_folders_deleted", "удалено {count} папок");
        m.insert("actions_no_folders", "папок нет");
        m.insert("actions_folders_get_error", "ошибка получения папок: {error}");
        m.insert("actions_folders_req_error", "ошибка запроса папок: {error}");
        m.insert("actions_leaving_channels", "отписка от каналов...");
        m.insert("actions_channels_left", "отписано от {count} каналов");
        m.insert("actions_hide_phone", "скрытие номера телефона...");
        m.insert("actions_phone_hidden", "номер телефона скрыт");
        m.insert("actions_phone_hide_error", "ошибка скрытия номера: {error}");
        m.insert("actions_hide_online", "скрытие статуса в сети...");
        m.insert("actions_online_hidden", "статус в сети скрыт");
        m.insert("actions_online_hide_error", "ошибка скрытия статуса: {error}");
        m.insert("actions_set_ttl", "установка Account TTL: {days} дней...");
        m.insert("actions_ttl_set", "Account TTL: {days} дней");
        m.insert("actions_ttl_error", "ошибка Account TTL: {error}");
        m.insert("actions_set_session_ttl", "установка Session TTL: {days} дней...");
        m.insert("actions_session_ttl_set", "Session TTL: {days} дней");
        m.insert("actions_session_ttl_error", "ошибка Session TTL: {error}");
        m.insert("actions_reset_2fa", "запрос сброса 2FA...");
        m.insert("actions_reset_2fa_sent", "запрос на сброс 2FA отправлен");
        m.insert("actions_reset_2fa_error", "сброс 2FA ошибка: {error}");
        m.insert("actions_set_2fa", "установка 2FA...");
        m.insert("actions_2fa_set", "2FA установлен");
        m.insert("actions_2fa_already_set", "2FA уже установлен, пропуск");
        m.insert("actions_2fa_error", "2FA ошибка: {error}");
        m.insert("actions_logout", "выход из аккаунта...");
        m.insert("actions_logged_out", "вышел");

        // Inviter extended
        m.insert("inviter_no_accounts", "не выбраны аккаунты");
        m.insert("inviter_no_target", "не указана целевая группа");
        m.insert("inviter_task_name", "Инвайтер: {count} акк.");
        m.insert("inviter_imported_usernames", "Импортировано в БД: {count} юзернеймов");
        m.insert("inviter_imported_phones", "Импортировано в БД: {count} телефонов");
        m.insert("inviter_stats_db", "БД статистики: {path}");
        m.insert("inviter_collecting_ids", "Сбор user_id аккаунтов...");
        m.insert("inviter_uid_error", "не удалось получить user_id для {id}: {error}");
        m.insert("inviter_admin_setup", "Главный аккаунт настроен, админки выданы: {count}");
        m.insert("inviter_admin_grant_error", "не удалось выдать админку, пропуск: {error}");
        m.insert("inviter_total_summary", "Итого приглашено (по БД): {count}");
        m.insert("inviter_target_resolved", "цель: id={id} ({target})");
        m.insert("inviter_not_a_group", "это канал, а не группа. Инвайтинг в канал возможен только через режим «Через админку».");
        m.insert("inviter_no_users", "нет пользователей для инвайта");
        m.insert("inviter_queue_size", "пользователей в очереди: {count}");
        m.insert("inviter_peer_flood_limit", "достигнут лимит PEER_FLOOD ({count}/{limit}), остановка");
        m.insert("inviter_already_in_group", "пропуск user_id={uid}: уже в группе");
        m.insert("inviter_user_added", "добавлен user_id={uid} ({done}/{max})");
        m.insert("inviter_not_confirmed", "user_id={uid} не подтверждён в группе");
        m.insert("inviter_revoke_admin_error", "ошибка снятия админки user_id={uid}: {error}");
        m.insert("inviter_user_invited", "приглашён user_id={uid} ({done}/{max})");
        m.insert("inviter_not_confirmed_after", "user_id={uid} не подтверждён в группе после инвайта");
        m.insert("inviter_force_no_users", "Force mode: нет больше пользователей для инвайта");
        m.insert("inviter_force_remaining", "Force mode: осталось пригласить {needed}, доступно pending: {pending}");
        m.insert("inviter_total_invited", "итого приглашено: {count}");
        m.insert("inviter_left_channel", "вышел из канала");
        m.insert("inviter_leave_error", "ошибка выхода: {error}");
        m.insert("inviter_peer_flood_user", "PEER_FLOOD user_id={uid} ({count}/lim)");
        m.insert("inviter_skip_user", "пропуск user_id={uid}: {error}");
        m.insert("inviter_too_many_channels", "пропуск user_id={uid}: слишком много каналов");
        m.insert("inviter_user_error", "ошибка user_id={uid}: {error}");
        m.insert("inviter_parse_error", "@{username}: не удалось распарсить");
        m.insert("inviter_resolve_error", "resolve @{username}: {error}");
        m.insert("inviter_import_contacts", "импорт контактов: найдено {found}/{total}");
        m.insert("inviter_import_contacts_error", "ошибка importContacts: {error}");
        m.insert("inviter_main_prefix", "[Главный]");
        m.insert("inviter_no_target_groups", "нет целевых групп");
        m.insert("inviter_target_channel", "целевой канал: id={id} title=\"{title}\"");
        m.insert("inviter_unknown_uid", "неизвестен user_id для {id}, пропуск");
        m.insert("inviter_admin_granted", "выдана админка user_id={uid}");
        m.insert("inviter_admin_error", "ошибка админки user_id={uid}: {error}");
        m.insert("inviter_no_uid", "нет user_id");
        m.insert("inviter_revoke_nobody", "некого отзывать");
        m.insert("inviter_main_uid_error", "не удалось получить user_id главного: {error}");
        m.insert("inviter_revoking", "отзыв админок...");
        m.insert("inviter_revoke_error", "ошибка отзыва user_id={uid}: {error}");
        m.insert("inviter_revoked", "отозвано админок: {done}/{total}");

        // First Comment extended

        // First Comment extended
        m.insert("first_comment_no_accounts", "не выбраны аккаунты");
        m.insert("first_comment_task_name", "Первонах: {count} акк.");
        m.insert("first_comment_no_channels_assigned", "нет назначенных каналов — поток закрыт");
        m.insert("first_comment_not_subscribed", "аккаунт не подписан на каналы — поток закрыт");
        m.insert("first_comment_no_channels", "нет каналов для мониторинга");
        m.insert("first_comment_monitoring_count", "мониторю {count} каналов");
        m.insert("first_comment_stopped", "остановлено");
        m.insert("first_comment_new_post_detail", "новый пост ch={ch} id={id}: \"{text}\"");
        m.insert("first_comment_comment_sent_post", "комментарий отправлен к посту {id}");
        m.insert("first_comment_comment_error", "ошибка комментария к посту {id}: {error}");
        m.insert("first_comment_channels_found", "найдено {count} каналов в подписках");
        m.insert("first_comment_channel_resolved", "канал: {title} (id={id})");
        m.insert("first_comment_resolve_error", "не удалось резолвить {target}: {error}");
        m.insert("first_comment_no_discussion", "не найдена группа обсуждения — у канала могут быть отключены комментарии");
        m.insert("first_comment_no_msg_id", "не найден msg_id в обсуждении");
        m.insert("first_comment_spamblock_warning", "⚠️ Внимание: {count} из выбранных аккаунтов имеют спамблок");
        m.insert("first_comment_banned_in_channel", "аккаунт заблокирован или имеет спамблок в канале ch={ch} — пропуск");
        m.insert("first_comment_spamblock_skip", "аккаунт имеет спамблок ({status}) — комментарий может быть ограничен");

        // Interceptor
        m.insert("interceptor_no_accounts", "не выбраны аккаунты");
        m.insert("interceptor_task_name", "Перехватчик: {count} акк.");
        m.insert("interceptor_collecting_ids", "Сбор user_id аккаунтов...");
        m.insert("interceptor_uid_error", "не удалось получить user_id для {id}: {error}");
        m.insert("interceptor_main_setup", "Главный аккаунт настроен, админки выданы: {count}");
        m.insert("interceptor_main_error", "ОШИБКА главного аккаунта: {error}");
        m.insert("interceptor_admin_skip", "[{idx}/{total}] не удалось выдать админку, пропуск потока: {error}");
        m.insert("interceptor_thread_error", "[{idx}/{total}] ОШИБКА: {error}");
        m.insert("interceptor_no_uid", "нет user_id");
        m.insert("interceptor_main_prefix", "[Главный]");
        m.insert("interceptor_admin_granted", "выдана админка user_id={uid} в {dest}");
        m.insert("interceptor_admin_error", "ошибка админки user_id={uid} в {dest}: {error}");
        m.insert("interceptor_target_channel", "целевой канал: id={id} title=\"{title}\"");
        m.insert("interceptor_joined_channel", "присоединился к каналу");
        m.insert("interceptor_assign_error", "ошибка назначения {dest}: {error}");
        m.insert("interceptor_no_dest_joined", "не удалось войти ни в один канал назначения");
        m.insert("interceptor_unknown_uid", "неизвестен user_id для {wid}, пропуск админки");
        m.insert("interceptor_admin_granted_id", "выдана админка user_id={uid} в id={id}");
        m.insert("interceptor_admin_error_id", "ошибка админки user_id={uid} в id={id}: {error}");
        m.insert("interceptor_nobody_revoke", "некого отзывать");
        m.insert("interceptor_main_uid_error", "не удалось получить user_id главного аккаунта: {error}");
        m.insert("interceptor_revoking", "отзыв админок...");
        m.insert("interceptor_revoke_error", "ошибка отзыва админки user_id={uid}: {error}");
        m.insert("interceptor_revoked", "отозвано админок: {count}");
        m.insert("interceptor_joined", "вступил в {target} (id={id})");
        m.insert("interceptor_needs_request", "{target} требует заявки — пропуск");
        m.insert("interceptor_join_failed", "не удалось вступить в {target}: {error}");
        m.insert("interceptor_no_groups_joined", "ни в одну группу/канал не удалось вступить");
        m.insert("interceptor_joined_dest", "вступил в назначение {dest} (id={id})");
        m.insert("interceptor_join_dest_failed", "не удалось вступить в назначение {dest}: {error}");
        m.insert("interceptor_no_dest_joined2", "ни в один канал назначения не удалось вступить");
        m.insert("interceptor_no_keywords", "нет ключевых слов");
        m.insert("interceptor_monitoring", "мониторинг {count} групп/каналов...");
        m.insert("interceptor_stopped", "остановлено");
        m.insert("interceptor_fatal_error", "фатальная ошибка: {error}");
        m.insert("interceptor_intercepted", "перехвачено msg_id={msg_id} в channel_id={channel_id} от {sender}");
        m.insert("interceptor_forward_error", "ошибка пересылки в id={id}: {error}");
        m.insert("interceptor_send_error", "ошибка отправки в id={id}: {error}");
        m.insert("interceptor_forwarded", "переслано в {sent}/{total} назначений ({text})");
        m.insert("interceptor_total", "итого перехвачено: {count}");

        // Accounts commands
        m.insert("acc_invalid_authkey", "Неверный формат auth_key (ожидается hex, 512 символов)");
        m.insert("acc_authkey_len", "auth_key должен быть 256 байт (512 hex символов), получено {bytes} байт");
        m.insert("acc_dc_range", "dc_id должен быть от 1 до 5");
        m.insert("acc_dc_detect_fail", "Не удалось определить DC. Auth key невалиден или все DC недоступны.");
        m.insert("acc_session_write_error", "Ошибка записи сессии: {error}");
        m.insert("acc_proxy_exists", "Этот прокси уже добавлен");
        m.insert("acc_read_file_error", "Не удалось прочитать файл: {error}");
        m.insert("acc_2fa_present", "Есть");
        m.insert("acc_2fa_until", "Есть до {date}");
        m.insert("acc_aging_years_months", "{years} г {months} мес");
        m.insert("acc_aging_years", "{years} г");
        m.insert("acc_aging_months", "{months} мес");
        m.insert("acc_aging_less_month", "< 1 мес");
        m.insert("acc_no_proxies_distribute", "Нет доступных прокси для распределения");
        m.insert("acc_session_not_found", "Файл сессии не найден");
        m.insert("acc_userid_empty", "user_id не заполнен — сначала запустите проверку аккаунтов (валидацию), чтобы определить user_id");
        m.insert("browser_chrome_download_error", "Не удалось скачать Chrome: {error}");
        m.insert("browser_chrome_extract_error", "Не удалось распаковать Chrome: {error}");
        m.insert("browser_chrome_not_found_after_extract", "Chrome не найден после распаковки");
        m.insert("browser_chrome_spawn_error", "Не удалось запустить Chrome: {error}");
        m.insert("browser_cdp_timeout", "Таймаут подключения к Chrome DevTools");
        m.insert("browser_cdp_connect_error", "Не удалось подключиться к Chrome DevTools: {error}");
        m.insert("browser_cdp_send_error", "Ошибка отправки команды CDP: {error}");
        m.insert("browser_proxy_bind_error", "Не удалось запустить локальный прокси: {error}");
        m.insert("browser_proxy_unsupported", "Неподдерживаемый тип прокси: {scheme}");

        // Account statuses
        m.insert("status_clean", "Без ограничений");
        m.insert("status_invalid", "Невалид");
        m.insert("status_frozen", "Заморожен");
        m.insert("status_perm_spam", "Вечный спамблок");
        m.insert("status_geo_spam", "Спамблок по ГЕО");
        m.insert("status_unchecked", "Не проверен");
        m.insert("status_checking", "Проверка...");
        m.insert("status_tdata", "TData (не конвертирован)");

        // User Lookup
        m.insert("user_lookup_no_accounts", "не выбраны аккаунты");
        m.insert("user_lookup_task_name", "Информация о пользователях ({count} акк.)");
        m.insert("user_lookup_no_input_file", "не указан входной файл");
        m.insert("user_lookup_read_file_error", "не удалось прочитать файл: {error}");
        m.insert("user_lookup_file_empty", "файл пуст или не содержит валидных строк");
        m.insert("user_lookup_duplicates_skipped", "Пропущено дублей: {count}");
        m.insert("user_lookup_targets_loaded", "Загружено {total} целей, {accounts} аккаунтов");
        m.insert("user_lookup_open_output_error", "открыть файл результата ({path}): {error}");
        m.insert("user_lookup_account_error", "ОШИБКА (аккаунт {idx}): {error}");
        m.insert("user_lookup_result", "Готово: найдено={found}, не найдено={not_found}, файл: {path}");
        m.insert("user_lookup_progress_phone", "[{idx}/{total}] телефон {target}...");
        m.insert("user_lookup_progress_username", "[{idx}/{total}] @{target}...");
        m.insert("user_lookup_not_found", "  не найден: {error}");
        m.insert("user_lookup_found", "  найден: {first_name} {last_name} (@{username})");
        m.insert("user_lookup_username_not_exists", "юзернейм @{username} не существует");
        m.insert("user_lookup_channel_skip", "@{username} — это канал/группа, пропускаем");
        m.insert("user_lookup_phone_not_registered", "номер +{phone} не зарегистрирован в Telegram");
        m.insert("user_lookup_phone_invalid", "номер +{phone} невалиден");
        m.insert("user_lookup_is_channel", "это канал/группа, пропускаем");
        m.insert("user_lookup_resolve_username_error", "resolveUsername: {error}");
        m.insert("user_lookup_resolve_phone_error", "resolvePhone: {error}");
        m.insert("user_lookup_newbot_exhausted", "исчерпаны попытки /newbot");
        m.insert("user_lookup_no_botfather_reply", "не удалось получить ответ от BotFather на /newbot");

        // Forwarder
        m.insert("forwarder_no_accounts", "не выбраны аккаунты");
        m.insert("forwarder_task_name", "Пересыльщик: {count} акк.");
        m.insert("forwarder_no_group", "не указана группа");
        m.insert("forwarder_start", "Запуск: {total} аккаунтов, группа: {group}");
        m.insert("forwarder_connect_error", "не удалось подключить: {error}");
        m.insert("forwarder_connected", "подключён");
        m.insert("forwarder_resolve_error", "не удалось разрезолвить группу: {error}");
        m.insert("forwarder_group_resolved", "группа разрезолвлена: id={id}");
        m.insert("forwarder_getstate_parse_error", "getState parse: {error}");
        m.insert("forwarder_getstate_error", "getState: {error}");
        m.insert("forwarder_resend_old", "пересылка старых непрочитанных ЛС...");
        m.insert("forwarder_old_forwarded", "старые ЛС переслано: {count}");
        m.insert("forwarder_stopped", "остановлено");
        m.insert("forwarder_fatal_error", "фатальная ошибка: {error}");
        m.insert("forwarder_subscribed", "подписался на группу");
        m.insert("forwarder_subscribe_failed_perm", "Не удалось подписаться на группу.");
        m.insert("forwarder_subscribe_failed", "не удалось подписаться на группу: {error}");
        m.insert("forwarder_forwarded", "переслано в группу (msg_id={id})");
        m.insert("forwarder_forwarded_no_id", "переслано, но не удалось извлечь msg_id");
        m.insert("forwarder_write_forbidden", "аккаунт не может писать в группу (нет прав)");
        m.insert("forwarder_forward_error", "ошибка пересылки: {error}");
        m.insert("forwarder_reply_copied", "ответ скопирован в ЛС user_id={user_id}");
        m.insert("forwarder_copy_error", "ошибка копирования: {error}");
        m.insert("forwarder_leave_error", "ошибка отписки: {error}");
        m.insert("forwarder_left_group", "отписался от группы");

        // Channel Creator
        m.insert("channelcreator_task_name", "Создание каналов: {count} акк.");
        m.insert("channelcreator_not_enough_titles", "недостаточно названий: {available} в файле, нужно до {needed}");
        m.insert("channelcreator_not_enough_usernames", "недостаточно юзернеймов: {available} в файле, нужно до {needed}");
        m.insert("channelcreator_db_open_error", "открыть БД: {error}");
        m.insert("channelcreator_db_tables_error", "создать таблицы: {error}");
        m.insert("channelcreator_creating", "{prefix} создаю {count} {entity_type} (тип: {channel_type})");
        m.insert("channelcreator_entity_channels", "каналов");
        m.insert("channelcreator_entity_groups", "групп");
        m.insert("channelcreator_titles_exhausted", "{prefix} названия закончились");
        m.insert("channelcreator_creating_title", "{prefix} [{idx}/{total}] создание: {title}");
        m.insert("channelcreator_created_id", "{prefix} создан id={id}");
        m.insert("channelcreator_photo_error", "{prefix} ошибка фото: {error}");
        m.insert("channelcreator_username_set", "{prefix} юзернейм: @{username}");
        m.insert("channelcreator_username_error", "{prefix} ошибка юзернейма: {error}");
        m.insert("channelcreator_invite_error", "{prefix} ошибка invite: {error}");
        m.insert("channelcreator_profile_error", "{prefix} ошибка профиля: {error}");
        m.insert("channelcreator_admin_error", "{prefix} ошибка добавления админа @{username}: {error}");
        m.insert("channelcreator_forward_error", "{prefix} ошибка forward: {error}");
        m.insert("channelcreator_post_error", "{prefix} ошибка поста: {error}");
        m.insert("channelcreator_forward_link_error", "не удалось распарсить ссылку на пост: {link}");
        m.insert("channelcreator_usernames_exhausted", "юзернеймы закончились");
        m.insert("channelcreator_username_attempts_exhausted", "не удалось подобрать юзернейм за 5 попыток");

        // Bot Creator
        m.insert("botcreator_task_name", "Создание ботов: {count} акк.");
        m.insert("botcreator_not_enough_names", "недостаточно имён: {available} в файле, нужно до {needed} ({accounts}×{max}). Уменьшите max или добавьте строки.");
        m.insert("botcreator_not_enough_usernames", "недостаточно юзернеймов: {available} в файле, нужно до {needed} ({accounts}×{max}). Уменьшите max или добавьте строки.");
        m.insert("botcreator_too_many_warning", "⚠️ С минимумом {min} × {accounts} акк = {total} ботов. Это может занять очень много времени.");
        m.insert("botcreator_starting", "{prefix} начинаю создание ботов...");
        m.insert("botcreator_start_error", "{prefix} ошибка отправки /start BotFather: {error}");
        m.insert("botcreator_resolve_flood", "{prefix} FLOOD_WAIT на resolve BotFather, ждём {seconds} сек...");
        m.insert("botcreator_resolve_flood_skip", "{prefix} FLOOD_WAIT {seconds} сек превышает лимит ({limit}), пропуск аккаунта");
        m.insert("botcreator_names_exhausted", "{prefix} названия закончились");
        m.insert("botcreator_usernames_exhausted", "{prefix} юзернеймы закончились");
        m.insert("botcreator_creating", "{prefix} [{idx}/{total}] создание бота: {name} (@{username})");
        m.insert("botcreator_newbot_error", "{prefix} ошибка отправки /newbot: {error}");
        m.insert("botcreator_rate_limit_skip", "{prefix} BotFather rate limit на {seconds} сек, пропускаем аккаунт");
        m.insert("botcreator_restricted", "{prefix} BotFather: аккаунту запрещено создавать ботов, пропуск");
        m.insert("botcreator_rate_limit_wait", "{prefix} rate limit, ждём {seconds} сек... (попытка {attempt}/3)");
        m.insert("botcreator_rate_limit_exhausted", "{prefix} исчерпаны попытки /newbot после rate limit");
        m.insert("botcreator_rate_limit_error", "{prefix} ошибка: BotFather rate limit, пропускаю аккаунт");
        m.insert("botcreator_username_taken", "{prefix} юзернейм занят, пробую: @{username}");
        m.insert("botcreator_created", "{prefix} бот создан, токен: {token_start}...{token_end}");
        m.insert("botcreator_token_error", "{prefix} [{idx}/{total}] ошибка: не удалось получить токен. BotFather: {reason}");
        m.insert("botcreator_cleanup_done", "{prefix} BotFather заблокирован, история удалена");
        m.insert("botcreator_db_open_error", "открыть БД: {error}");
        m.insert("botcreator_db_tables_error", "создать таблицы: {error}");
        m.insert("botcreator_newbot_exhausted", "исчерпаны попытки /newbot");
        m.insert("botcreator_no_botfather_reply", "не удалось получить ответ от BotFather на /newbot");
        // Bot Parser
        m.insert("bot_parser_no_accounts", "No accounts selected");
        m.insert("bot_parser_task_name", "Bot parser: {count} acc.");
        m.insert("bot_parser_db_path", "Output DB: {path}");
        m.insert("bot_parser_account_error", "{prefix} account error: {error}");
        m.insert("bot_parser_collecting", "{prefix} collecting bot list...");
        m.insert("bot_parser_no_bots", "{prefix} \u{443} \u{430}\u{43a}\u{43a}\u{430}\u{443}\u{43d}\u{442}\u{430} \u{43d}\u{435}\u{442} \u{431}\u{43e}\u{442}\u{43e}\u{432}");
        m.insert("bot_parser_found", "{prefix} found bots: {count}");
        m.insert("bot_parser_revoke", "{prefix} [{idx}/{total}] regenerating @{username}");
        m.insert("bot_parser_token", "{prefix} [{idx}/{total}] getting token for @{username}");
        m.insert("bot_parser_bot_error", "{prefix} @{username}: {error}");
        m.insert("bot_parser_cleanup", "{prefix} BotFather blocked, history deleted");
        m.insert("bot_parser_result", "{prefix} done: bots {bots}, tokens {tokens}");
        m.insert("bot_parser_flood_wait", "{prefix} FLOOD_WAIT {seconds} sec exceeds limit ({limit}), skipping account");
        m.insert("bot_parser_flood_wait_wait", "{prefix} FLOOD_WAIT {seconds} sec, waiting...");
        m.insert("bot_parser_db_open_error", "open DB: {error}");
        m.insert("bot_parser_db_tables_error", "create tables: {error}");
        // Checker
        m.insert("checker_checking_accounts", "Проверка {count} аккаунтов...");
        m.insert("checker_no_accounts", "Нет аккаунтов для проверки.");
        m.insert("checker_session_read_error", "ОШИБКА чтения .session: {error}");
        m.insert("checker_no_accounts_in_tdata", "Нет аккаунтов в tdata: {path}");
        m.insert("checker_multi_account_tdata", "Мульти-аккаунт tdata ({count} акк.): {path}");
        m.insert("checker_local_passcode", "Tdata защищена Local Passcode, пропущена: {path}");
        m.insert("checker_parse_error", "ОШИБКА парсинга: {error}");
        m.insert("checker_checking_progress", "[{idx}/{total}] Проверка...");
        m.insert("checker_authkey_invalid_size", "  ОШИБКА: auth_key неверного размера");
        m.insert("checker_connect_error", "  ОШИБКА подключения (5 попыток): {error}");
        m.insert("checker_invalid", "  НЕВАЛИД: {error}");
        m.insert("checker_valid", "ВАЛИД id={id}");
        m.insert("checker_nft_tag", "    NFT-тег: @{tag}");
        m.insert("checker_short_channel_tag", "    Короткий тег канала: @{username} ({title})");
        m.insert("checker_channel", "    Канал: {title} ({count} подп.)");
        m.insert("checker_group", "    Группа: {title} ({count} участн.)");
        m.insert("checker_channel_balance", "    {kind} «{title}»: Stars={stars} TON={ton}");
        m.insert("checker_seed", "    Seed: {text}");
        m.insert("checker_downloaded", "    Скачан: {path}");
        m.insert("checker_download_error", "    Ошибка скачивания {filename}: {error}");
        m.insert("checker_added_to_panel", "    Добавлен в панель: {id}");
        m.insert("checker_role_premium", "Премиум");
        m.insert("checker_role_default", "Чекер");
        m.insert("checker_summary", "Готово. Валид: {valid} | Невалид: {invalid}");

        // Stories extended
        m.insert("stories_read_error", "не удалось прочитать {path}: {error}");
        m.insert("stories_read_media_error", "не удалось прочитать медиафайл: {error}");
        m.insert("stories_no_media", "не указаны медиафайлы");
        m.insert("stories_read_tags_error", "не удалось прочитать файл тегов: {error}");
        m.insert("stories_tags_empty", "файл с юзернеймами пуст");
        m.insert("stories_task_name", "Истории: {count} акк.");
        m.insert("stories_done", "[{idx}/{total}] выполнено: {msg}");
        m.insert("stories_error", "[{idx}/{total}] ошибка: {error}");
        m.insert("stories_stopped", "остановлено");
        m.insert("stories_caption_too_long", "описание слишком длинное, теги не влезут");
        m.insert("stories_caption_no_tag_fit", "описание слишком длинное, ни один тег не влезет");
        m.insert("stories_caption_over_limit", "описание превышает лимит ({limit} символов)");
        m.insert("stories_premium_required", "аккаунт без Premium — истории недоступны");
        m.insert("stories_flood_wait", "FLOOD_WAIT {secs} сек (лимит {limit})");
        m.insert("stories_uploaded", "{count} историй загружено{tags}");
        m.insert("stories_tags_suffix", ", {count} тегов");

        // Global Search extended
        m.insert("global_search_no_accounts", "не выбраны аккаунты");
        m.insert("global_search_task_name", "Глобальный поиск");
        m.insert("global_search_no_input_file", "не указан входной файл");
        m.insert("global_search_read_error", "не удалось прочитать файл: {error}");
        m.insert("global_search_skipped_invalid", "Пропущено невалидных слов: {count}");
        m.insert("global_search_file_empty", "файл пуст или не содержит валидных слов");
        m.insert("global_search_loaded", "Загружено {words} слов, {accounts} аккаунтов, режим: {mode}, раздача: {distribution}, searchGlobal: {sg}");
        m.insert("global_search_open_file_error", "открыть файл: {error}");
        m.insert("global_search_results_db", "БД результатов: {path}");
        m.insert("global_search_thread_connect_error", "Поток {idx}: не удалось подключить: {error}");
        m.insert("global_search_thread_no_words", "Поток {idx}: нет слов для обработки");
        m.insert("global_search_thread_words", "Поток {idx}: получено {count} слов");
        m.insert("global_search_word_result", "[{idx}/{total}] «{word}» — найдено {found} (новых: {new})");
        m.insert("global_search_result", "Готово: найдено уникальных={count}, файл: {path}");
        m.insert("global_search_mode_channels", "каналы");
        m.insert("global_search_mode_groups", "группы");
        m.insert("global_search_mode_users", "пользователи");
        m.insert("global_search_mode_all", "все");
        m.insert("global_search_yes", "да");
        m.insert("global_search_no", "нет");

        // Link Checker
        m.insert("link_checker_no_accounts", "не выбраны аккаунты");
        m.insert("link_checker_task_name", "Валидность ссылок");
        m.insert("link_checker_no_input_file", "не указан входной файл");
        m.insert("link_checker_read_error", "не удалось прочитать файл: {error}");
        m.insert("link_checker_file_empty", "файл пуст или не содержит ссылок");
        m.insert("link_checker_loaded", "Загружено {links} ссылок, {accounts} аккаунтов (потоков)");
        m.insert("link_checker_no_output_file", "не указан выходной файл");
        m.insert("link_checker_db_open_error", "открыть БД: {error}");
        m.insert("link_checker_db_tables_error", "создать таблицы: {error}");
        m.insert("link_checker_thread_connect_error", "Поток {idx}: не удалось подключить аккаунт: {error}");
        m.insert("link_checker_valid", "[{idx}/{total}] {link} — валидна ({kind}: {name})");
        m.insert("link_checker_invalid", "[{idx}/{total}] {link} — невалидна");
        m.insert("link_checker_skipped", "[{idx}/{total}] {link} — пропущена: {reason}");
        m.insert("link_checker_result", "Готово: валидных={valid}, невалидных={invalid}, пропущено={skipped}, БД: {path}");
        m.insert("link_checker_retry_limit", "превышен лимит попыток");

        // Cloner extended
        m.insert("cloner_task_name", "Клонер: {id}");
        m.insert("cloner_source", "Источник: {title} (id={id})");
        m.insert("cloner_destination", "Назначение: id={id}");
        m.insert("cloner_sweep_error", "Не удалось подчистить служебные сообщения: {error}");
        m.insert("cloner_left_source", "Вышли из канала-источника");
        m.insert("cloner_leave_source_error", "Не удалось выйти из источника: {error}");
        m.insert("cloner_stats", "Готово: скопировано={copied} пропущено={skipped} ошибок={errors}");
        m.insert("cloner_from_gt_to", "from_id больше to_id");
        m.insert("cloner_no_messages", "Нет сообщений в указанном диапазоне");
        m.insert("cloner_collected", "Собрано постов: {count} (id {lo}..={hi})");
        m.insert("cloner_skipped_service", "пропущено: msg={id} (служебное)");
        m.insert("cloner_skipped_reason", "пропущено: msg={id} ({reason})");
        m.insert("cloner_skipped_media", "пропущено: msg={id} ({reason})");
        m.insert("cloner_skipped_size", "пропущено: msg={id} ({reason}, {kb} КБ)");
        m.insert("cloner_error_msg", "ошибка: msg={id} {error}");
        m.insert("cloner_copied_msg", "скопировано: msg={id} -> dst={dst}");
        m.insert("cloner_sweep_count", "Подчищено служебных сообщений: {count}");
        m.insert("cloner_sweep_result", "Очистка служебных сообщений вернула: {error}");

        // Inviter extra (remaining hardcoded)
        m.insert("inviter_no_target_groups_err", "нет целевых групп");
        m.insert("inviter_target_channel_info", "целевой канал: id={id} title=\"{title}\"");
        m.insert("inviter_unknown_uid_skip", "неизвестен user_id для {id}, пропуск");
        m.insert("inviter_admin_granted_msg", "выдана админка user_id={uid}");
        m.insert("inviter_admin_error_msg", "ошибка админки user_id={uid}: {error}");
        m.insert("inviter_no_uid_err", "нет user_id");
        m.insert("inviter_revoke_nobody_msg", "некого отзывать");
        m.insert("inviter_main_uid_error_msg", "не удалось получить user_id главного: {error}");
        m.insert("inviter_revoking_msg", "отзыв админок...");
        m.insert("inviter_revoke_error_msg", "ошибка отзыва user_id={uid}: {error}");
        m.insert("inviter_revoked_msg", "отозвано админок: {done}/{total}");

        // Username Checker extended
        m.insert("uchecker_task_name", "Чекер юзернеймов");
        m.insert("uchecker_no_input_file", "не указан входной файл");
        m.insert("uchecker_read_file_error", "не удалось прочитать файл: {error}");
        m.insert("uchecker_invalid_short", "@{name} — тег невалиден (менее 4 символов), пропуск");
        m.insert("uchecker_invalid_chars", "@{name} — тег невалиден (недопустимые символы), пропуск");
        m.insert("uchecker_skipped_invalid", "Пропущено невалидных/дублей: {count}");
        m.insert("uchecker_file_empty", "файл пуст или не содержит валидных юзернеймов");
        m.insert("uchecker_no_proxies", "нет прокси — для чекера юзернеймов нужны прокси");
        m.insert("uchecker_loaded", "Загружено {usernames} юзернеймов, {proxies} прокси, {threads} потоков");
        m.insert("uchecker_open_db_error", "открыть БД: {error}");
        m.insert("uchecker_create_table_error", "создать таблицу: {error}");
        m.insert("uchecker_error_user", "[{idx}/{total}] @{name} ошибка: {error}");
        m.insert("uchecker_progress", "[{idx}/{total}] @{name} — {status}");
        m.insert("uchecker_status_free", "свободен");
        m.insert("uchecker_status_taken", "занят");
        m.insert("uchecker_status_for_sale", "продаётся на Fragment");
        m.insert("uchecker_status_sold", "продан");
        m.insert("uchecker_status_error", "ошибка");
        m.insert("uchecker_autoclaim", "Автозанятие: {free} свободных тегов, {accounts} аккаунтов");
        m.insert("uchecker_claimed", "  @{name} — занят на аккаунте {phone}");
        m.insert("uchecker_claim_failed", "  @{name} — не удалось занять: {error}");
        m.insert("uchecker_result", "Готово: свободно={free}, занято={taken}, файл: {path}");

        // Masslooking extended
        m.insert("masslooking_no_accounts", "не выбраны аккаунты");
        m.insert("masslooking_task_name", "Масслукинг: {count} акк.");
        m.insert("masslooking_targets_count", "целей: {count}");
        m.insert("masslooking_stories_error", "ошибка историй user={user_id}: {error}");
        m.insert("masslooking_stories_viewed", "просмотрено {count} историй user={user_id}");
        m.insert("masslooking_reaction_sent", "реакция {emoji} на историю user={user_id}");
        m.insert("masslooking_reaction_error", "ошибка реакции user={user_id}: {error}");
        m.insert("masslooking_reply_sent", "ответ отправлен user={user_id}");
        m.insert("masslooking_reply_error", "ошибка ответа user={user_id}: {error}");
        m.insert("masslooking_processed", "итого обработано: {count}");
        m.insert("masslooking_inbox_found", "найдено {count} пользователей из входящих");
        m.insert("masslooking_chat_found", "найдено {count} пользователей из чата");

        // Actions extended (remaining hardcoded)
        m.insert("actions_error_generic", "ОШИБКА: {error}");
        m.insert("actions_prefix_too_long", "username_prefix слишком длинный: {chars} символов (макс. 30)");

        // Cloner transform (skip reasons)
        m.insert("cloner_skip_keyword", "содержит стоп-слово");
        m.insert("cloner_skip_documents", "документы отключены");
        m.insert("cloner_skip_photos", "фото отключены");
        m.insert("cloner_skip_videos", "видео отключены");
        m.insert("cloner_skip_video_msg", "сообщения с видео отключены");
        m.insert("cloner_skip_ext_link", "внешние ссылки отключены");
        m.insert("cloner_skip_tg_link", "telegram-ссылки отключены");
        m.insert("cloner_skip_file_size", "файл больше лимита");
        m.insert("cloner_skip_video_size", "видео больше лимита");
        m.insert("cloner_skip_photo_size", "фото больше лимита");

        // Converter pyro
        m.insert("converter_pyro_open_error", "Не удалось открыть Pyrogram сессию: {error}");
        m.insert("converter_pyro_table_error", "Pyrogram: таблица sessions повреждена: {error}");
        m.insert("converter_pyro_empty_table", "Pyrogram: пустая или битая sessions таблица: {error}");
        m.insert("converter_pyro_authkey_size", "Pyrogram: auth_key неверного размера ({bytes} байт)");
        m.insert("converter_pyro_create_error", "Не удалось создать Pyrogram сессию: {error}");
        m.insert("converter_pyro_schema_error", "Pyrogram: ошибка создания схемы: {error}");
        m.insert("converter_pyro_write_error", "Pyrogram: ошибка записи sessions: {error}");
        m.insert("converter_pyro_version_error", "Pyrogram: ошибка записи version: {error}");

        // Converter telethon
        m.insert("converter_telethon_open_session_error", "Не удалось открыть файл сессии. Возможно он повреждён или имеет неизвестный формат. ({error})");
        m.insert("converter_telethon_table_error", "Файл сессии повреждён: таблица sessions не найдена. ({error})");
        m.insert("converter_telethon_empty_error", "Файл сессии пуст или повреждён. ({error})");
        m.insert("converter_telethon_create_error", "Не удалось создать файл сессии: {error}");
        m.insert("converter_telethon_schema_error", "Ошибка создания схемы: {error}");
        m.insert("converter_telethon_version_error", "Ошибка записи версии: {error}");
        m.insert("converter_telethon_write_error", "Ошибка записи сессии: {error}");

        // Queue validate
        m.insert("validate_task_name", "Проверка {count} аккаунтов");
        m.insert("validate_checking_attempt", "Проверка (попытка {attempt}/5)");
        m.insert("validate_restrictions", "Проверка ограничений...");
        m.insert("validate_2fa_unknown", "Неизвестен");
        m.insert("validate_2fa_hint", "Неизвестен, подсказка: {hint}");

        // MTProto invite
        m.insert("invite_empty_link", "пустая ссылка");
        m.insert("invite_addlist_not_channel", "addlist-ссылка не является каналом или группой");
        m.insert("invite_parse_error", "не удалось распарсить ссылку");
        m.insert("invite_already_member_no_hash", "уже участник, но в ответе нет access_hash — попробуйте ссылку на канал");
        m.insert("invite_request_needed", "канал «{label}» требует подтверждения админа (заявка на вступление). Подайте заявку и дождитесь одобрения вручную.");
        m.insert("invite_request_sent", "канал требует подтверждения админа — заявка отправлена. Продолжить нельзя без одобрения.");
        m.insert("invite_already_no_hash", "уже участник, но access_hash не получен");

        // MTProto client
        m.insert("mtproto_reconnecting", "переподключение...");
        m.insert("mtproto_network_error", "ошибка сети (попытка {attempt}/5): {error}, реконнект через {delay}мс...");
        m.insert("mtproto_flood_over_limit", "FLOOD_WAIT: {wait} сек > лимит {limit} сек, аборт");
        m.insert("mtproto_flood_waiting", "FLOOD_WAIT: ожидание {wait} сек...");

        // Account session
        m.insert("session_read_json_error", "Не удалось прочитать .json файл: {error}");
        m.insert("session_parse_json_error", "Ошибка парсинга .json файла: {error}");
        m.insert("session_serialize_error", "Ошибка сериализации: {error}");
        m.insert("session_write_json_error", "Не удалось записать .json файл: {error}");

        // Accounts reauth
        m.insert("reauth_status", "Переавторизация...");
        m.insert("reauth_step", "Переавторизация ({step}/3)");
        m.insert("reauth_signing_in", "Переавторизация (вход)");

        // Accounts auth_login
        m.insert("auth_connect_error", "Не удалось подключиться к серверам Telegram. {error}");
        m.insert("auth_dh_error", "Ошибка DH key exchange: {error}");
        m.insert("auth_2fa_not_set", "Двухфакторная аутентификация не установлена");
        m.insert("auth_srp_error", "Ошибка вычисления SRP: {error}");
        m.insert("auth_session_expired", "Сессия истекла или не найдена");
        m.insert("auth_session_write_error", "Ошибка записи сессии: {error}");

        // Cloner destination
        m.insert("cloner_dest_parse_error", "не удалось разобрать ID/ссылку назначения");
        m.insert("cloner_dest_numeric_unsupported", "укажите @username или ссылку на канал-приёмник; чистый ID без access_hash не поддерживается");
        m.insert("cloner_dest_username_taken", "юзернейм @{username} занят или невалиден");
        m.insert("cloner_dest_avatar_copied", "Аватар скопирован");
        m.insert("cloner_dest_avatar_error", "Ошибка копирования аватара: {error}");
        m.insert("cloner_dest_no_avatar", "У источника не удалось извлечь аватар (или его нет) — пропускаем копирование");

        // Cloner config
        m.insert("cloner_cfg_public_no_username", "публичный канал — нужен юзернейм");
        m.insert("cloner_cfg_no_existing_id", "не указан ID существующего канала");

        // Proxy
        m.insert("proxy_no_available", "нет доступных прокси, добавьте прокси или включите работу без прокси в настройках");
        m.insert("proxy_connect_error", "Не удалось подключиться к серверам Telegram. Проверьте прокси на валидность. ({error})");
        m.insert("proxy_validate_task", "Проверка {count} прокси");

        // LLM
        m.insert("llm_not_configured", "LLM не настроен (проверьте Настройки -> LLM)");
        m.insert("llm_specify_url_token", "укажите API URL и токен");

        // Accounts connect
        m.insert("connect_session_error", "сессия: {error}");
        m.insert("connect_invalid_authkey", "невалидный auth_key");
        // Checker validate
        m.insert("checker_validate_authkey_size", "auth_key неверного размера");
        m.insert("checker_validate_connect", "Подключение: {error}");

        // Quick Actions
        m.insert("quick_mailing", "Новая рассылка");
        m.insert("quick_checker", "Чекнуть аккаунты");
        m.insert("quick_inviter", "Инвайт в группу");
        m.insert("quick_parser", "Парсер участников");

        // Checker task
        m.insert("checker_task_name", "Чекер: {count} аккаунтов");

        // Inviter DB
        m.insert("inviter_db_open_error", "открыть БД пользователей: {error}");
        m.insert("inviter_db_create_tables", "создать таблицы пользователей: {error}");
        m.insert("inviter_stats_db_open_error", "открыть БД статистики: {error}");
        m.insert("inviter_stats_db_create_tables", "создать таблицы статистики: {error}");

        // First Comment LLM
        m.insert("first_comment_post_prefix", "Пост: {text}");

        // Converter tdata
        m.insert("converter_tdata_no_userid", "user_id=0 — невозможно создать валидную TData без user_id");

        // MTProto transport
        m.insert("mtproto_connect_error", "Не удалось подключиться к серверам Telegram. Проверьте прокси на валидность. ({error})");

        // Account actions
        m.insert("actions_2fa_already_set_err", "2FA уже установлен");
        m.insert("actions_2fa_no_algo_params", "2FA: сервер не вернул параметры алгоритма");
        m.insert("actions_2fa_srp_stale", "2FA: параметры SRP устарели, пропуск");
        m.insert("actions_response_too_small", "ответ слишком маленький, вероятно не изображение");
        m.insert("actions_db_open_error", "открыть БД действий: {error}");

        // Two-FA display values
        m.insert("two_fa_unknown", "Неизвестен");
        m.insert("two_fa_unknown_set", "Установлен, неизвестен");
        m.insert("two_fa_unknown_hint", "Неизвестен, подсказка: {hint}");

        // Role display values
        m.insert("role_premium", "Премиум");
        m.insert("role_checker", "Чекер");

        m
    };
    
    static ref EN: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();
        // Common
        m.insert("done", "Done");
        m.insert("error", "ERROR");
        m.insert("stopped_by_user", "Stopped by user");
        m.insert("db_open_error", "open DB: {error}");
        m.insert("db_create_tables_error", "create tables: {error}");
        
        // Parser
        m.insert("parser_collected", "collected: {count}");
        m.insert("parser_group_done", "group processed: {name}");
        m.insert("parser_joining", "joining {name}...");
        m.insert("parser_leaving", "leaving {name}...");
        m.insert("parser_flood_wait", "FLOOD_WAIT {seconds} sec, waiting...");
        m.insert("parser_flood_skip", "FLOOD_WAIT {seconds} sec > limit, skipping");
        m.insert("parser_no_accounts", "no accounts selected");
        m.insert("parser_no_targets", "no targets specified for parsing");
        m.insert("parser_task_name", "Parser: {groups} groups, {accounts} acc.");
        m.insert("parser_start", "Start: {groups} groups, {accounts} accounts, {threads} threads");
        m.insert("parser_db_path", "DB: {path}");
        m.insert("parser_thread_start", "starting: {target}");
        m.insert("parser_thread_done", "done: +{count} users");
        m.insert("parser_export_txt", "Exporting TXT: {path}...");
        m.insert("parser_txt_exported", "TXT exported: {path}");
        m.insert("parser_total", "Total: {done} groups processed, {total} unique users");
        m.insert("parser_no_filter", "no member filter selected");
        m.insert("parser_prefix", "[acc {acc}/group {group}/{total}]");
        m.insert("parser_target", "target: id={id} {hint}");
        m.insert("parser_target_private", "(private)");
        m.insert("parser_broadcast_not_group", "this is a broadcast channel, not a group");
        m.insert("parser_no_admin_rights", "no administrator rights");
        m.insert("parser_access_check", "access check: {error}");
        m.insert("parser_admins_loaded", "Admins loaded: {count}");
        m.insert("parser_admins_error", "Could not get admin list: {error}");
        m.insert("parser_method", "Method: {method}");
        m.insert("parser_method_alphabet", "alphabet search ({count} chars)");
        m.insert("parser_method_pagination", "direct pagination (empty query)");
        m.insert("parser_parse_error", "Parse error (char='{char}' offset={offset}): {error}");
        m.insert("parser_channel_private_wait", "ChannelPrivate — waiting 45 sec...");
        m.insert("parser_flood_wait_short", "FLOOD_WAIT {seconds} sec — waiting...");
        m.insert("parser_members_hidden", "member list is hidden");
        m.insert("parser_error", "Error: {error}");
        m.insert("parser_char_progress", "[{done}/{total}] char='{char}': +{added} (collected: {collected}, viewed: {viewed})");
        m.insert("parser_collected_progress", "Collected: {count}");
        m.insert("parser_msg_mode_days", "Message mode: collecting authors from last {days} days");
        m.insert("parser_msg_mode_all", "Message mode: collecting authors from full history");
        m.insert("parser_flood_over_limit", "FLOOD_WAIT {seconds} > limit — stopping");
        m.insert("parser_get_history_error", "getHistory error: {error}");
        m.insert("parser_msg_history_end", "End of message history");
        m.insert("parser_days_limit", "Reached {days} day limit — stopping");
        m.insert("parser_msg_progress", "Messages: {messages}, unique collected: {collected}");
        m.insert("parser_msg_total", "Total messages: {messages}, unique authors: {collected}");
        m.insert("parser_comment_mode_days", "Comment mode: depth {days} days");
        m.insert("parser_comment_mode_all", "Comment mode: full history");
        m.insert("parser_posts_history_end", "End of post history");
        m.insert("parser_posts_progress", "Posts: {scanned}, with comments: {with_comments}, collected: {collected}");
        m.insert("parser_days_limit_short", "Reached {days} day limit");
        m.insert("parser_posts_total", "Total posts: {scanned}, with comments: {with_comments}, collected: {collected}");
        
        // Inviter
        m.insert("inviter_invited", "invited: {username}");
        m.insert("inviter_added", "added: {username}");
        m.insert("inviter_peer_flood", "PEER_FLOOD: account stopped");
        m.insert("inviter_flood_wait", "FLOOD_WAIT {seconds} sec");
        m.insert("inviter_autostop", "autostop: error limit reached");
        
        // Boost
        m.insert("boost_done", "done: {detail}");
        m.insert("boost_bot_started", "bot started: {name}");
        m.insert("boost_subscribed", "subscribed: {channel}");
        m.insert("boost_reacted", "reaction sent");
        m.insert("boost_viewed", "viewed");
        m.insert("boost_task_name", "Boost: {count} acc.");
        m.insert("boost_no_bot_links", "no bot links specified");
        m.insert("boost_bot_error", "{prefix} bot error {link}: {error}");
        m.insert("boost_empty_bot_link", "empty bot link");
        m.insert("boost_bot_activated", "{prefix} bot @{username} activated, done");
        m.insert("boost_bot_activated_ref", "{prefix} bot @{username} activated with ref='{payload}', done");
        m.insert("boost_bot_blocked", "{prefix} bot @{username} blocked and deleted");
        m.insert("boost_no_posts_to_view", "no posts to view");
        m.insert("boost_views_done", "{prefix} viewed {count} posts in '{channel}', done");
        m.insert("boost_join_failed", "{prefix} could not join channel: {error}");
        m.insert("boost_no_reaction_links", "no links for reactions");
        m.insert("boost_no_posts_for", "{prefix} {label}: no posts");
        m.insert("boost_joined", "{prefix} joined {label}");
        m.insert("boost_join_failed_label", "{prefix} could not join {label}: {error}");
        m.insert("boost_resolve_error", "{prefix} resolve error {link}: {error}");
        m.insert("boost_no_posts_for_reactions", "no available posts for reactions");
        m.insert("boost_reactions_db", "{prefix} reactions DB: {path}");
        m.insert("boost_no_emoji_skip", "{prefix} no emoji selected, skipping");
        m.insert("boost_reaction_progress", "{prefix} {emoji} {label}/{msg_id} reaction {idx} [{ok}/{total}]");
        m.insert("boost_reaction_limit", "{prefix} reaction limit on {label}/{msg_id}");
        m.insert("boost_reaction_error", "{prefix} error {label}/{msg_id}: {error}");
        m.insert("boost_reactions_done", "{prefix} reactions done: ok={ok}, errors={errs}");
        m.insert("boost_no_reactions_sent", "no reactions were sent");
        m.insert("boost_invalid_post_link", "invalid post link");
        m.insert("boost_empty_channel_link", "empty channel link");
        m.insert("boost_join_link_done", "{prefix} join via link {link} done");
        m.insert("boost_archive_error", "{prefix} archive error: {error}");
        m.insert("boost_archived", "{prefix} moved to archive");
        m.insert("boost_empty_folder_links", "empty folder links list");
        m.insert("boost_not_addlist", "{prefix} error: '{link}' does not look like t.me/addlist/");
        m.insert("boost_check_invite_error", "{prefix} checkChatlistInvite '{slug}' error: {error}");
        m.insert("boost_parse_invite_error", "{prefix} parse invite '{slug}' error: {error}");
        m.insert("boost_folder_imported", "{prefix} folder '{name}' imported, done");
        m.insert("boost_join_invite_error", "{prefix} joinChatlistInvite '{slug}' error: {error}");
        m.insert("boost_no_folders_imported", "none of {count} folders imported");
        m.insert("boost_parse_link_error", "could not parse link: {link}");
        m.insert("boost_sub_username_failed", "could not subscribe via username");
        m.insert("boost_addlist_unsupported", "addlist link not supported in this mode");
        m.insert("boost_unknown_link_type", "unknown link type: {kind}");
        m.insert("boost_left_channel", "{prefix} left channel");
        m.insert("boost_leave_error", "{prefix} leave error: {error}");
        m.insert("boost_reactions_db_open_error", "open reactions DB: {error}");
        
        // Cloner
        m.insert("cloner_copied", "copied: post #{id}");
        m.insert("cloner_skipped", "skipped: post #{id}");
        m.insert("cloner_channel_created", "channel created: {name}");
        
        // Auto-Reply
        m.insert("auto_reply_sent", "reply sent: {username}");
        m.insert("auto_reply_skipped", "skipped: {reason}");
        m.insert("auto_reply_voice_sent", "voice sent: {username}");
        m.insert("auto_reply_ban_word", "ban word: skipped");
        m.insert("auto_reply_no_accounts", "no accounts selected");
        m.insert("auto_reply_task_name", "Auto-Reply: {count} acc.");
        m.insert("auto_reply_audio_convert_error", "Audio conversion error: {error}");
        m.insert("auto_reply_voice_read_error", "Voice file read error: {error}");
        m.insert("auto_reply_image_read_error", "Image read error: {error}");
        m.insert("auto_reply_video_read_error", "Video read error: {error}");
        m.insert("auto_reply_db_create_error", "DB creation error: {error}");
        m.insert("auto_reply_db_open_error", "open DB: {error}");
        m.insert("auto_reply_db_tables_error", "create tables: {error}");
        m.insert("auto_reply_connected", "connected (user_id={user_id})");
        m.insert("auto_reply_uploading_image", "uploading image...");
        m.insert("auto_reply_image_uploaded", "image uploaded");
        m.insert("auto_reply_uploading_video", "uploading video...");
        m.insert("auto_reply_video_uploaded", "video uploaded");
        m.insert("auto_reply_listening", "listening for incoming messages...");
        m.insert("auto_reply_stopped", "stopped");
        m.insert("auto_reply_limit_reached", "reached limit of {limit} replies");
        m.insert("auto_reply_get_diff_error", "getDifference error: {error}");
        m.insert("auto_reply_parse_diff_error", "parse diff error: {error}");
        m.insert("auto_reply_no_access_hash", "skipped: could not get access_hash for user_id={user_id}");
        m.insert("auto_reply_not_in_list", "skipped (not in list): user_id={user_id}");
        m.insert("auto_reply_ban_word_skip", "skipped (ban word): user_id={user_id}");
        m.insert("auto_reply_voice_not_loaded", "voice file not loaded");
        m.insert("auto_reply_invalid_forward_id", "invalid message ID for forwarding");
        m.insert("auto_reply_voice_label", "voice sent");
        m.insert("auto_reply_forward_label", "forwarded");
        m.insert("auto_reply_reply_label", "reply sent");
        m.insert("auto_reply_send_error", "send error user_id={user_id}: {error}");
        
        // First Comment
        m.insert("first_comment_sent", "comment sent: {channel}");
        m.insert("first_comment_monitoring", "monitoring channels...");
        m.insert("first_comment_new_post", "new post in {channel}");
        
        // Reporter
        m.insert("reporter_sent", "report sent: {target}");
        m.insert("reporter_db_path", "DB: {path}");
        m.insert("reporter_db_create_error", "Failed to create DB: {error}");
        m.insert("reporter_task_name", "Reporter: {count} acc.");
        m.insert("reporter_username_invalid", "username @{target} is invalid");
        m.insert("reporter_username_not_exists", "username @{target} does not exist");
        m.insert("reporter_target_info", "target: @{target} (id={id}, {kind})");
        m.insert("reporter_target_channel", "channel");
        m.insert("reporter_target_user", "user");
        m.insert("reporter_report_sent", "report sent [{idx}/{total}] reason={reason}");
        m.insert("reporter_error", "error: {error}");
        m.insert("reporter_invalid_link", "invalid post/channel link");
        m.insert("reporter_invalid_channel_id", "invalid channel ID");
        m.insert("reporter_no_posts", "{target}: no posts to report");
        m.insert("reporter_target_posts", "target: {target} ({count} posts)");
        m.insert("reporter_channel_report_sent", "report {target}/{msg_id} [{idx}/{total}] reason={reason}");
        m.insert("reporter_error_sub", "error (sub): {error}");
        m.insert("reporter_error_comment", "error (comment): {error}");
        m.insert("reporter_bot_resolved", "bot resolved: id={id}");
        m.insert("reporter_bot_report_sent", "report sent [{idx}/{total}] query='{query}'");
        m.insert("reporter_bot_blocked", "bot blocked, history deleted");
        m.insert("reporter_photo_target", "photo-report: @{target} (id={id})");
        m.insert("reporter_photo_fetch_error", "error fetching photos: {error}");
        m.insert("reporter_no_photos", "@{target}: no profile photos");
        m.insert("reporter_photos_count", "profile photos: {count}");
        m.insert("reporter_photo_report_sent", "photo-report [{idx}/{total}] reason={reason}");
        m.insert("reporter_photo_report_error", "photo-report error: {error}");
        m.insert("reporter_db_open_error", "open reporter DB: {error}");
        m.insert("reporter_db_tables_error", "create tables: {error}");
        
        // Masslooking
        m.insert("masslooking_viewed", "viewed: {username}");
        m.insert("masslooking_reacted", "reacted: {username}");
        m.insert("masslooking_replied", "replied: {username}");
        
        // Mailing
        m.insert("mailing_sent", "sent: {target}");
        m.insert("mailing_ban", "account banned");
        m.insert("mailing_spamblock", "spamblock");
        m.insert("mailing_skip_spamblock", "skip: spamblock ({status})");
        m.insert("mailing_peer_flood_retry", "PEER_FLOOD — one more attempt...");
        m.insert("mailing_peer_flood_stop", "PEER_FLOOD twice — stopping thread");
        m.insert("mailing_no_accounts", "no accounts selected");
        m.insert("mailing_task_name", "Mailing: {count} acc.");
        m.insert("mailing_waiting", "waiting until {time} ({ms} ms)...");
        m.insert("mailing_repost_group_created", "auto-repost: group created");
        m.insert("mailing_repost_msg_sent", "auto-repost: message sent to temp group");
        m.insert("mailing_repost_no_msg_id", "auto-repost: could not get msg_id");
        m.insert("mailing_repost_send_error", "auto-repost: send error: {error}");
        m.insert("mailing_repost_no_chat_id", "auto-repost: could not parse chat_id");
        m.insert("mailing_repost_create_error", "auto-repost: group creation error: {error}");
        m.insert("mailing_sent_to_chat", "sent to {target}");
        m.insert("mailing_send_error_chat", "send error to {target}");
        m.insert("mailing_no_posts_in", "no posts in {target}");
        m.insert("mailing_comment_sent", "comment in {target} on post {msg_id}");
        m.insert("mailing_comment_error", "comment error {target}: {error}");
        m.insert("mailing_no_phones", "no phone numbers for mailing");
        m.insert("mailing_phone_resolve_error", "could not resolve number: {phone}");
        m.insert("mailing_no_story_link", "no story link specified");
        m.insert("mailing_story_parse_error", "could not parse story link: {link}");
        m.insert("mailing_story_forwarded", "story forwarded -> @{username}");
        m.insert("mailing_story_forward_error", "forward error @{username}: {error}");
        m.insert("mailing_story_owner_resolve_error", "could not resolve story owner: {username}");
        m.insert("mailing_repost_group_deleted", "auto-repost: temp group deleted");
        m.insert("mailing_total_sent", "total sent: {count}");
        m.insert("mailing_sent_user", "sent user_id={user_id}");
        m.insert("mailing_error_user", "error user_id={user_id}: {error}");
        
        // Stories
        m.insert("stories_published", "published");
        
        // Warmer
        m.insert("warmer_action_done", "action done: {action}");
        m.insert("warmer_no_accounts", "no accounts selected");
        m.insert("warmer_task_name", "Warmer: {count} acc.");
        m.insert("warmer_starting", "Starting warmup on {count} accounts");
        m.insert("warmer_acc_error", "[acc {idx}/{total}] ERROR: {error}");
        m.insert("warmer_no_uid_warning", "WARNING: could not determine user_id, fake_chat unavailable");
        m.insert("warmer_spamblock_json", "spamblock from JSON: {status}");
        m.insert("warmer_spamblock_detected", "SPAMBLOCK: {status}");
        m.insert("warmer_no_spamblock", "no spamblock detected");
        m.insert("warmer_spamblock_peer_flood", "SPAMBLOCK (PEER_FLOOD)");
        m.insert("warmer_temp_tag_set", "temp tag set @{username}");
        m.insert("warmer_no_actions", "no enabled actions");
        m.insert("warmer_duration", "running for {minutes} min");
        m.insert("warmer_until_stop", "running until stopped");
        m.insert("warmer_saved", "saved: \"{text}\"");
        m.insert("warmer_saved_no_msgid", "could not extract msg_id from saved response");
        m.insert("warmer_del_msg_error", "delete msg {id} error: {error}");
        m.insert("warmer_saved_send_error", "saved message send error: {error}");
        m.insert("warmer_search", "search: {word}");
        m.insert("warmer_search_error", "search error: {error}");
        m.insert("warmer_read_channel", "reading channel: search '{word}'");
        m.insert("warmer_contacts_search_error", "contacts.search error: {error}");
        m.insert("warmer_reaction", "reaction {emoji} on post {id}");
        m.insert("warmer_reactions_disabled", "reactions disabled for channel {id}");
        m.insert("warmer_read_dialogs", "reading dialogs");
        m.insert("warmer_read_dialogs_error", "dialog read error: {error}");
        m.insert("warmer_view_stories", "viewing stories");
        m.insert("warmer_stories_read_error", "stories read error: {error}");
        m.insert("warmer_stories_viewed", "viewed {count} stories from channel {id}");
        m.insert("warmer_subscribing", "subscribing to channel from search");
        m.insert("warmer_search_result", "contacts.search '{word}': len={len}");
        m.insert("warmer_search_empty_fallback", "contacts.search empty, fallback resolve @{name}");
        m.insert("warmer_resolve_error", "resolve @{name} error: {error}");
        m.insert("warmer_parse_resolve_error", "parse_resolved_peer error: {error}");
        m.insert("warmer_unsubscribe_error", "unsubscribe from old channel {id} error: {error}");
        m.insert("warmer_subscribed", "subscribed to channel id={id}");
        m.insert("warmer_join_error", "joinChannel {id} error: {error}");
        m.insert("warmer_no_channel_found", "could not find channel via search or resolve");
        m.insert("warmer_group_members", "browsing group members");
        m.insert("warmer_group_members_count", "group id={id}: {count} members");
        m.insert("warmer_photo_error", "photo load error user {id}: {error}");
        m.insert("warmer_avatars_viewed", "viewed {count} avatars");
        m.insert("warmer_del_contact_error", "delete contact {id} error: {error}");
        m.insert("warmer_contact_added_group", "added contact from group id={id}");
        m.insert("warmer_resolve_username_error", "resolve username '{username}' error: {error}");
        m.insert("warmer_skip_fake_chat", "skip fake_chat: could not resolve peer (user={id}, username='{username}')");
        m.insert("warmer_llm_error", "LLM error: {error}, fallback to random");
        m.insert("warmer_fake_chat_msg", "fake chat -> user {id}: \"{text}\"");
        m.insert("warmer_send_error", "send error: {error}");
        m.insert("warmer_spamblock_on_send", "spamblock on send: {error}");
        m.insert("warmer_too_many_send_errors", "too many send errors, possible spamblock");
        m.insert("warmer_fake_chat_no_peers", "fake chat (no peers): saved \"{text}\"");
        m.insert("warmer_read_telegram", "reading Telegram chat");
        m.insert("warmer_read_telegram_error", "read 777000 error: {error}");
        m.insert("warmer_add_contact", "adding contact from search");
        m.insert("warmer_contact_added", "added contact id={id}");
        m.insert("warmer_contact_add_error", "add contact {id} error: {error}");
        m.insert("warmer_del_old_contact_error", "delete old contact {id} error: {error}");
        m.insert("warmer_search_no_users", "search returned no users");
        m.insert("warmer_rest", "resting {seconds} sec...");
        m.insert("warmer_cleanup_start", "cleanup start: channels={channels}, saved_msgs={msgs}, fake_chats={chats}, contacts={contacts}");
        m.insert("warmer_cleanup_unsub", "cleanup: unsubscribing from {count} channels");
        m.insert("warmer_cleanup_unsub_error", "unsubscribe from {id} error: {error}");
        m.insert("warmer_cleanup_saved", "cleanup: deleting ~{count} saved messages");
        m.insert("warmer_cleanup_saved_found", "found {count} messages in saved, deleting");
        m.insert("warmer_cleanup_saved_no_ids", "could not get message IDs from saved");
        m.insert("warmer_cleanup_saved_history_error", "saved history fetch error: {error}");
        m.insert("warmer_cleanup_chats", "cleanup: deleting chats with {count} peers");
        m.insert("warmer_cleanup_chat_error", "delete history with {id} error: {error}");
        m.insert("warmer_cleanup_contacts", "cleanup: deleting {valid} contacts (of {total} tracked)");
        m.insert("warmer_cleanup_contacts_error", "delete contacts error: {error}");
        m.insert("warmer_cleanup_contacts_skip", "cleanup: {count} contacts with access_hash=0, skipping");
        m.insert("warmer_cleanup_tag_error", "temp tag removal error: {error}");
        m.insert("warmer_cleanup_tag_done", "temp tag removed");
        m.insert("warmer_session_done", "session complete ({count} actions)");
        
        // Channel Creator
        m.insert("channel_created", "created: {name}");
        
        // Bot Creator
        m.insert("bot_created", "bot created: {username}");
        m.insert("bot_token", "token: {token}");
        
        // Global Search
        m.insert("search_found", "new: {count}");
        
        // Username Checker
        m.insert("username_free", "free: {username}");
        m.insert("username_taken", "taken: {username}");
        m.insert("username_fragment", "fragment: {username}");
        
        // Converter
        m.insert("converter_ok", "converted: {path}");
        m.insert("converter_err", "error: {path}");
        m.insert("converter_unknown_format", "unknown format: {format}");
        m.insert("converter_telethon_read", "[telethon] read: {path}");
        m.insert("converter_telethon_error", "[telethon] error {path}: {error}");
        m.insert("converter_pyrogram_read", "[pyrogram] read: {path}");
        m.insert("converter_pyrogram_error", "[pyrogram] error {path}: {error}");
        m.insert("converter_tdata_not_found", "[tdata] no tdata found in {path}");
        m.insert("converter_tdata_read", "[tdata] read {count} acc. from {path}");
        m.insert("converter_tdata_error", "[tdata] error {path}: {error}");
        m.insert("converter_tdatazip_unpack_error", "[tdata_zip] unpack error {path}: {error}");
        m.insert("converter_tdatazip_read", "[tdata_zip] read {count} acc. from {path}");
        m.insert("converter_tdatazip_error", "[tdata_zip] tdata error {path}: {error}");
        m.insert("converter_authkey_read_error", "[authkey] read error {path}: {error}");
        m.insert("converter_authkey_invalid_hex", "[authkey] {path} line {line}: invalid hex");
        m.insert("converter_authkey_no_dc", "[authkey] {path} line {line}: could not determine DC");
        m.insert("converter_authkey_read", "[authkey] read {count} keys from {path}");
        m.insert("converter_no_userid", "user_id=0 — cannot create TData without user_id. Add to panel and validate first.");
        m.insert("converter_tdatazip_unsupported_out", "TdataZip format not supported as output");
        m.insert("converter_open_file_error", "could not open file: {error}");
        m.insert("converter_write_error", "write error: {error}");
        m.insert("converter_tdatazip_unsupported_target", "Target format TdataZip is not supported");
        m.insert("converter_reading_paths", "Reading {count} paths from format {format}...");
        m.insert("converter_accounts_found", "Found {count} accounts to convert");
        m.insert("converter_outdir_error", "Could not create output folder: {error}");
        m.insert("converter_added_to_panel", "    added to panel id={id}");
        m.insert("converter_not_added_to_panel", "    not added to panel: {error}");
        m.insert("converter_summary", "Done. Success: {ok} | Errors: {err}");
        m.insert("converter_ok_short", "OK");
        m.insert("llm_rephrase_prompt", "Rephrase this message slightly with different words, keep the meaning and length. No quotes, no emoji. Don't add anything from yourself.");
        m.insert("converter_unpack_over_limit", "unpacked >1 GB, aborted");
        m.insert("converter_read_error", "Read error {path}: {error}");
        m.insert("converter_invalid_authkey_len", "Line {line}: invalid auth_key length ({bytes} bytes)");
        m.insert("converter_invalid_hex_line", "Line {line}: invalid hex");
        m.insert("converter_all_complete", "All {count} lines already have authkey:dc_id, writing unchanged.");
        m.insert("converter_detecting_dc", "Detecting DC for {count} keys ({threads} threads)...");
        m.insert("converter_line_dc", "  line {line}: DC={dc}");
        m.insert("converter_line_no_dc", "  line {line}: DC not determined, skipped");
        m.insert("converter_lines_written", "Wrote {count} lines to {path}");
        m.insert("converter_write_error_short", "Write error: {error}");
        m.insert("converter_task_name", "Converter");

        // Account Actions
        m.insert("actions_working_on_accounts", "Working on accounts: {count} acc.");
        m.insert("actions_not_enough_usernames", "Not enough usernames: {usable} in file, {total} accounts");
        m.insert("actions_db_path", "DB: {path}");
        m.insert("actions_account_dead", "ERROR: Account dead (session invalid)");
        m.insert("actions_delete_account", "deleting account...");
        m.insert("actions_account_deleted", "account deleted");
        m.insert("actions_delete_username", "deleting username...");
        m.insert("actions_username_deleted", "username deleted");
        m.insert("actions_username_not_set", "username not set");
        m.insert("actions_changing_username", "changing username to @{username}...");
        m.insert("actions_username_changed", "username changed: @{username}");
        m.insert("actions_username_unchanged", "username unchanged (already @{username})");
        m.insert("actions_username_unavailable", "@{username} unavailable, generating new...");
        m.insert("actions_username_unavail_err", "username @{username} unavailable: {error}");
        m.insert("actions_username_error", "username error: {error}");
        m.insert("actions_delete_avatars", "deleting avatars...");
        m.insert("actions_avatars_deleted", "deleted {count} avatars");
        m.insert("actions_no_avatars", "no avatars");
        m.insert("actions_photo_error", "photo error: {error}");
        m.insert("actions_photo_get_error", "photo fetch error: {error}");
        m.insert("actions_photo_dl_error", "auto-photo download error: {error}");
        m.insert("actions_photo_ul_error", "auto-photo upload error: {error}");
        m.insert("actions_delete_stories", "deleting all stories...");
        m.insert("actions_stories_error", "stories deletion error: {error}");
        m.insert("actions_stories_get_error", "stories fetch error: {error}");
        m.insert("actions_stories_deleted", "deleted {count} stories");
        m.insert("actions_no_stories", "no stories");
        m.insert("actions_set_photo", "setting photo...");
        m.insert("actions_photo_set", "photo set");
        m.insert("actions_gen_photo", "generating auto-photo...");
        m.insert("actions_autophoto_set", "auto-photo set");
        m.insert("actions_set_emoji_avatar", "setting emoji avatar...");
        m.insert("actions_emoji_avatar_set", "emoji avatar set");
        m.insert("actions_emoji_avatar_error", "emoji avatar error: {error}");
        m.insert("actions_emoji_get_error", "emoji fetch error: {error}");
        m.insert("actions_emoji_req_error", "emoji request error: {error}");
        m.insert("actions_emoji_no_valid", "failed to find valid emoji in 3 attempts");
        m.insert("actions_emoji_list_empty", "emoji list empty");
        m.insert("actions_changing_name", "changing name: {first} {last}...");
        m.insert("actions_name_set", "name: {first} {last}");
        m.insert("actions_set_bio", "changing bio...");
        m.insert("actions_bio_updated", "bio updated");
        m.insert("actions_delete_bio", "deleting bio...");
        m.insert("actions_bio_deleted", "bio deleted");
        m.insert("actions_set_birthday", "setting birthday: {d}.{m}.{y}...");
        m.insert("actions_birthday_set", "birthday set");
        m.insert("actions_delete_contacts", "deleting contacts...");
        m.insert("actions_contacts_deleted", "deleted {count} contacts");
        m.insert("actions_no_contacts", "no contacts");
        m.insert("actions_contacts_del_error", "contacts deletion error: {error}");
        m.insert("actions_contacts_get_error", "contacts fetch error: {error}");
        m.insert("actions_contacts_req_error", "contacts request error: {error}");
        m.insert("actions_delete_dialogs", "deleting all dialogs...");
        m.insert("actions_dialogs_deleted", "deleted {count} dialogs");
        m.insert("actions_dialogs_get_error", "dialogs fetch error: {error}");
        m.insert("actions_dialogs_parse_error", "dialogs parse error: {error}");
        m.insert("actions_delete_bot_dialogs", "deleting bot dialogs...");
        m.insert("actions_bots_deleted", "deleted and blocked {count} bots");
        m.insert("actions_read_dialogs", "reading all dialogs...");
        m.insert("actions_dialogs_read", "read {count} dialogs");
        m.insert("actions_read_error", "read dialogs error: {error}");
        m.insert("actions_delete_folders", "deleting folders...");
        m.insert("actions_folders_deleted", "deleted {count} folders");
        m.insert("actions_no_folders", "no folders");
        m.insert("actions_folders_get_error", "folders fetch error: {error}");
        m.insert("actions_folders_req_error", "folders request error: {error}");
        m.insert("actions_leaving_channels", "leaving channels...");
        m.insert("actions_channels_left", "left {count} channels");
        m.insert("actions_hide_phone", "hiding phone number...");
        m.insert("actions_phone_hidden", "phone number hidden");
        m.insert("actions_phone_hide_error", "phone hide error: {error}");
        m.insert("actions_hide_online", "hiding online status...");
        m.insert("actions_online_hidden", "online status hidden");
        m.insert("actions_online_hide_error", "online status hide error: {error}");
        m.insert("actions_set_ttl", "setting Account TTL: {days} days...");
        m.insert("actions_ttl_set", "Account TTL: {days} days");
        m.insert("actions_ttl_error", "Account TTL error: {error}");
        m.insert("actions_set_session_ttl", "setting Session TTL: {days} days...");
        m.insert("actions_session_ttl_set", "Session TTL: {days} days");
        m.insert("actions_session_ttl_error", "Session TTL error: {error}");
        m.insert("actions_reset_2fa", "requesting 2FA reset...");
        m.insert("actions_reset_2fa_sent", "2FA reset request sent");
        m.insert("actions_reset_2fa_error", "2FA reset error: {error}");
        m.insert("actions_set_2fa", "setting up 2FA...");
        m.insert("actions_2fa_set", "2FA set");
        m.insert("actions_2fa_already_set", "2FA already set, skipping");
        m.insert("actions_2fa_error", "2FA error: {error}");
        m.insert("actions_logout", "logging out...");
        m.insert("actions_logged_out", "logged out");

        // Inviter extended
        m.insert("inviter_no_accounts", "no accounts selected");
        m.insert("inviter_no_target", "no target group specified");
        m.insert("inviter_task_name", "Inviter: {count} acc.");
        m.insert("inviter_imported_usernames", "Imported to DB: {count} usernames");
        m.insert("inviter_imported_phones", "Imported to DB: {count} phones");
        m.insert("inviter_stats_db", "Stats DB: {path}");
        m.insert("inviter_collecting_ids", "Collecting account user_ids...");
        m.insert("inviter_uid_error", "failed to get user_id for {id}: {error}");
        m.insert("inviter_admin_setup", "Main account configured, admin rights granted: {count}");
        m.insert("inviter_admin_grant_error", "failed to grant admin rights, skipping: {error}");
        m.insert("inviter_total_summary", "Total invited (from DB): {count}");
        m.insert("inviter_target_resolved", "target: id={id} ({target})");
        m.insert("inviter_not_a_group", "this is a channel, not a group. Inviting to a channel requires admin mode.");
        m.insert("inviter_no_users", "no users to invite");
        m.insert("inviter_queue_size", "users in queue: {count}");
        m.insert("inviter_peer_flood_limit", "PEER_FLOOD limit reached ({count}/{limit}), stopping");
        m.insert("inviter_already_in_group", "skip user_id={uid}: already in group");
        m.insert("inviter_user_added", "added user_id={uid} ({done}/{max})");
        m.insert("inviter_not_confirmed", "user_id={uid} not confirmed in group");
        m.insert("inviter_revoke_admin_error", "admin revoke error user_id={uid}: {error}");
        m.insert("inviter_user_invited", "invited user_id={uid} ({done}/{max})");
        m.insert("inviter_not_confirmed_after", "user_id={uid} not confirmed in group after invite");
        m.insert("inviter_force_no_users", "Force mode: no more users to invite");
        m.insert("inviter_force_remaining", "Force mode: {needed} more to invite, {pending} pending available");
        m.insert("inviter_total_invited", "total invited: {count}");
        m.insert("inviter_left_channel", "left channel");
        m.insert("inviter_leave_error", "leave error: {error}");
        m.insert("inviter_peer_flood_user", "PEER_FLOOD user_id={uid} ({count}/lim)");
        m.insert("inviter_skip_user", "skip user_id={uid}: {error}");
        m.insert("inviter_too_many_channels", "skip user_id={uid}: too many channels");
        m.insert("inviter_user_error", "error user_id={uid}: {error}");
        m.insert("inviter_parse_error", "@{username}: failed to parse");
        m.insert("inviter_resolve_error", "resolve @{username}: {error}");
        m.insert("inviter_import_contacts", "import contacts: found {found}/{total}");
        m.insert("inviter_import_contacts_error", "importContacts error: {error}");
        m.insert("inviter_main_prefix", "[Main]");
        m.insert("inviter_no_target_groups", "no target groups");
        m.insert("inviter_target_channel", "target channel: id={id} title=\"{title}\"");
        m.insert("inviter_unknown_uid", "unknown user_id for {id}, skipping");
        m.insert("inviter_admin_granted", "admin rights granted user_id={uid}");
        m.insert("inviter_admin_error", "admin error user_id={uid}: {error}");
        m.insert("inviter_no_uid", "no user_id");
        m.insert("inviter_revoke_nobody", "nobody to revoke");
        m.insert("inviter_main_uid_error", "failed to get main account user_id: {error}");
        m.insert("inviter_revoking", "revoking admin rights...");
        m.insert("inviter_revoke_error", "revoke error user_id={uid}: {error}");
        m.insert("inviter_revoked", "revoked admin rights: {done}/{total}");

        // First Comment extended
        m.insert("first_comment_no_accounts", "no accounts selected");
        m.insert("first_comment_task_name", "First Comment: {count} acc.");
        m.insert("first_comment_no_channels_assigned", "no assigned channels — thread closed");
        m.insert("first_comment_not_subscribed", "account not subscribed to any channels — thread closed");
        m.insert("first_comment_no_channels", "no channels to monitor");
        m.insert("first_comment_monitoring_count", "monitoring {count} channels");
        m.insert("first_comment_stopped", "stopped");
        m.insert("first_comment_new_post_detail", "new post ch={ch} id={id}: \"{text}\"");
        m.insert("first_comment_comment_sent_post", "comment sent to post {id}");
        m.insert("first_comment_comment_error", "comment error for post {id}: {error}");
        m.insert("first_comment_channels_found", "found {count} channels in subscriptions");
        m.insert("first_comment_channel_resolved", "channel: {title} (id={id})");
        m.insert("first_comment_resolve_error", "failed to resolve {target}: {error}");
        m.insert("first_comment_no_discussion", "discussion group not found — channel comments may be disabled");
        m.insert("first_comment_no_msg_id", "msg_id not found in discussion");
        m.insert("first_comment_spamblock_warning", "⚠️ Warning: {count} of selected accounts have spamblock");
        m.insert("first_comment_banned_in_channel", "account banned or has spamblock in channel ch={ch} — skipping");
        m.insert("first_comment_spamblock_skip", "account has spamblock ({status}) — commenting may be restricted");

        // Interceptor
        m.insert("interceptor_no_accounts", "no accounts selected");
        m.insert("interceptor_task_name", "Interceptor: {count} acc.");
        m.insert("interceptor_collecting_ids", "Collecting account user_ids...");
        m.insert("interceptor_uid_error", "failed to get user_id for {id}: {error}");
        m.insert("interceptor_main_setup", "Main account configured, admin rights granted: {count}");
        m.insert("interceptor_main_error", "MAIN ACCOUNT ERROR: {error}");
        m.insert("interceptor_admin_skip", "[{idx}/{total}] failed to grant admin, skipping thread: {error}");
        m.insert("interceptor_thread_error", "[{idx}/{total}] ERROR: {error}");
        m.insert("interceptor_no_uid", "no user_id");
        m.insert("interceptor_main_prefix", "[Main]");
        m.insert("interceptor_admin_granted", "admin granted user_id={uid} in {dest}");
        m.insert("interceptor_admin_error", "admin error user_id={uid} in {dest}: {error}");
        m.insert("interceptor_target_channel", "target channel: id={id} title=\"{title}\"");
        m.insert("interceptor_joined_channel", "joined channel");
        m.insert("interceptor_assign_error", "assignment error {dest}: {error}");
        m.insert("interceptor_no_dest_joined", "could not join any destination channel");
        m.insert("interceptor_unknown_uid", "unknown user_id for {wid}, skipping admin");
        m.insert("interceptor_admin_granted_id", "admin granted user_id={uid} in id={id}");
        m.insert("interceptor_admin_error_id", "admin error user_id={uid} in id={id}: {error}");
        m.insert("interceptor_nobody_revoke", "nobody to revoke");
        m.insert("interceptor_main_uid_error", "failed to get main account user_id: {error}");
        m.insert("interceptor_revoking", "revoking admin rights...");
        m.insert("interceptor_revoke_error", "admin revoke error user_id={uid}: {error}");
        m.insert("interceptor_revoked", "admins revoked: {count}");
        m.insert("interceptor_joined", "joined {target} (id={id})");
        m.insert("interceptor_needs_request", "{target} requires a join request — skipping");
        m.insert("interceptor_join_failed", "could not join {target}: {error}");
        m.insert("interceptor_no_groups_joined", "could not join any group/channel");
        m.insert("interceptor_joined_dest", "joined destination {dest} (id={id})");
        m.insert("interceptor_join_dest_failed", "could not join destination {dest}: {error}");
        m.insert("interceptor_no_dest_joined2", "could not join any destination channel");
        m.insert("interceptor_no_keywords", "no keywords");
        m.insert("interceptor_monitoring", "monitoring {count} groups/channels...");
        m.insert("interceptor_stopped", "stopped");
        m.insert("interceptor_fatal_error", "fatal error: {error}");
        m.insert("interceptor_intercepted", "intercepted msg_id={msg_id} in channel_id={channel_id} from {sender}");
        m.insert("interceptor_forward_error", "forward error to id={id}: {error}");
        m.insert("interceptor_send_error", "send error to id={id}: {error}");
        m.insert("interceptor_forwarded", "forwarded to {sent}/{total} destinations ({text})");
        m.insert("interceptor_total", "total intercepted: {count}");

        // Accounts commands
        m.insert("acc_invalid_authkey", "Invalid auth_key format (expected hex, 512 chars)");
        m.insert("acc_authkey_len", "auth_key must be 256 bytes (512 hex chars), got {bytes} bytes");
        m.insert("acc_dc_range", "dc_id must be between 1 and 5");
        m.insert("acc_dc_detect_fail", "Could not determine DC. Auth key is invalid or all DCs are unreachable.");
        m.insert("acc_session_write_error", "Session write error: {error}");
        m.insert("acc_proxy_exists", "This proxy is already added");
        m.insert("acc_read_file_error", "Could not read file: {error}");
        m.insert("acc_2fa_present", "Set");
        m.insert("acc_2fa_until", "Set until {date}");
        m.insert("acc_aging_years_months", "{years}y {months}mo");
        m.insert("acc_aging_years", "{years}y");
        m.insert("acc_aging_months", "{months}mo");
        m.insert("acc_aging_less_month", "< 1 mo");
        m.insert("acc_no_proxies_distribute", "No available proxies to distribute");
        m.insert("acc_session_not_found", "Session file not found");
        m.insert("acc_userid_empty", "user_id is empty — run account validation first to determine user_id");
        m.insert("browser_chrome_download_error", "Could not download Chrome: {error}");
        m.insert("browser_chrome_extract_error", "Could not extract Chrome: {error}");
        m.insert("browser_chrome_not_found_after_extract", "Chrome not found after extraction");
        m.insert("browser_chrome_spawn_error", "Could not spawn Chrome: {error}");
        m.insert("browser_cdp_timeout", "Timeout connecting to Chrome DevTools");
        m.insert("browser_cdp_connect_error", "Could not connect to Chrome DevTools: {error}");
        m.insert("browser_cdp_send_error", "Error sending CDP command: {error}");
        m.insert("browser_proxy_bind_error", "Could not start local proxy: {error}");
        m.insert("browser_proxy_unsupported", "Unsupported proxy type: {scheme}");
        // Account statuses
        m.insert("status_clean", "No restrictions");
        m.insert("status_invalid", "Invalid");
        m.insert("status_frozen", "Frozen");
        m.insert("status_perm_spam", "Permanent spamblock");
        m.insert("status_geo_spam", "Geo spamblock");
        m.insert("status_unchecked", "Unchecked");
        m.insert("status_checking", "Checking...");
        m.insert("status_tdata", "TData (not converted)");

        // User Lookup
        m.insert("user_lookup_no_accounts", "no accounts selected");
        m.insert("user_lookup_task_name", "User info ({count} acc.)");
        m.insert("user_lookup_no_input_file", "no input file specified");
        m.insert("user_lookup_read_file_error", "could not read file: {error}");
        m.insert("user_lookup_file_empty", "file is empty or contains no valid lines");
        m.insert("user_lookup_duplicates_skipped", "Duplicates skipped: {count}");
        m.insert("user_lookup_targets_loaded", "Loaded {total} targets, {accounts} accounts");
        m.insert("user_lookup_open_output_error", "open output file ({path}): {error}");
        m.insert("user_lookup_account_error", "ERROR (account {idx}): {error}");
        m.insert("user_lookup_result", "Done: found={found}, not found={not_found}, file: {path}");
        m.insert("user_lookup_progress_phone", "[{idx}/{total}] phone {target}...");
        m.insert("user_lookup_progress_username", "[{idx}/{total}] @{target}...");
        m.insert("user_lookup_not_found", "  not found: {error}");
        m.insert("user_lookup_found", "  found: {first_name} {last_name} (@{username})");
        m.insert("user_lookup_username_not_exists", "username @{username} does not exist");
        m.insert("user_lookup_channel_skip", "@{username} is a channel/group, skipping");
        m.insert("user_lookup_phone_not_registered", "number +{phone} is not registered on Telegram");
        m.insert("user_lookup_phone_invalid", "number +{phone} is invalid");
        m.insert("user_lookup_is_channel", "this is a channel/group, skipping");
        m.insert("user_lookup_resolve_username_error", "resolveUsername: {error}");
        m.insert("user_lookup_resolve_phone_error", "resolvePhone: {error}");
        m.insert("user_lookup_newbot_exhausted", "/newbot attempts exhausted");
        m.insert("user_lookup_no_botfather_reply", "no response from BotFather to /newbot");

        // Forwarder
        m.insert("forwarder_no_accounts", "no accounts selected");
        m.insert("forwarder_task_name", "Forwarder: {count} acc.");
        m.insert("forwarder_no_group", "no group specified");
        m.insert("forwarder_start", "Start: {total} accounts, group: {group}");
        m.insert("forwarder_connect_error", "could not connect: {error}");
        m.insert("forwarder_connected", "connected");
        m.insert("forwarder_resolve_error", "could not resolve group: {error}");
        m.insert("forwarder_group_resolved", "group resolved: id={id}");
        m.insert("forwarder_getstate_parse_error", "getState parse: {error}");
        m.insert("forwarder_getstate_error", "getState: {error}");
        m.insert("forwarder_resend_old", "forwarding old unread PMs...");
        m.insert("forwarder_old_forwarded", "old PMs forwarded: {count}");
        m.insert("forwarder_stopped", "stopped");
        m.insert("forwarder_fatal_error", "fatal error: {error}");
        m.insert("forwarder_subscribed", "subscribed to group");
        m.insert("forwarder_subscribe_failed_perm", "Could not subscribe to group.");
        m.insert("forwarder_subscribe_failed", "could not subscribe to group: {error}");
        m.insert("forwarder_forwarded", "forwarded to group (msg_id={id})");
        m.insert("forwarder_forwarded_no_id", "forwarded, but could not extract msg_id");
        m.insert("forwarder_write_forbidden", "account cannot write to group (no permissions)");
        m.insert("forwarder_forward_error", "forward error: {error}");
        m.insert("forwarder_reply_copied", "reply copied to DM user_id={user_id}");
        m.insert("forwarder_copy_error", "copy error: {error}");
        m.insert("forwarder_leave_error", "leave error: {error}");
        m.insert("forwarder_left_group", "left group");

        // Channel Creator
        m.insert("channelcreator_task_name", "Create channels: {count} acc.");
        m.insert("channelcreator_not_enough_titles", "not enough titles: {available} in file, need up to {needed}");
        m.insert("channelcreator_not_enough_usernames", "not enough usernames: {available} in file, need up to {needed}");
        m.insert("channelcreator_db_open_error", "open DB: {error}");
        m.insert("channelcreator_db_tables_error", "create tables: {error}");
        m.insert("channelcreator_creating", "{prefix} creating {count} {entity_type} (type: {channel_type})");
        m.insert("channelcreator_entity_channels", "channels");
        m.insert("channelcreator_entity_groups", "groups");
        m.insert("channelcreator_titles_exhausted", "{prefix} titles exhausted");
        m.insert("channelcreator_creating_title", "{prefix} [{idx}/{total}] creating: {title}");
        m.insert("channelcreator_created_id", "{prefix} created id={id}");
        m.insert("channelcreator_photo_error", "{prefix} photo error: {error}");
        m.insert("channelcreator_username_set", "{prefix} username: @{username}");
        m.insert("channelcreator_username_error", "{prefix} username error: {error}");
        m.insert("channelcreator_invite_error", "{prefix} invite error: {error}");
        m.insert("channelcreator_profile_error", "{prefix} profile error: {error}");
        m.insert("channelcreator_admin_error", "{prefix} add admin @{username} error: {error}");
        m.insert("channelcreator_forward_error", "{prefix} forward error: {error}");
        m.insert("channelcreator_post_error", "{prefix} post error: {error}");
        m.insert("channelcreator_forward_link_error", "could not parse post link: {link}");
        m.insert("channelcreator_usernames_exhausted", "usernames exhausted");
        m.insert("channelcreator_username_attempts_exhausted", "could not find username in 5 attempts");

        // Bot Creator
        m.insert("botcreator_task_name", "Create bots: {count} acc.");
        m.insert("botcreator_not_enough_names", "not enough names: {available} in file, need up to {needed} ({accounts}×{max}). Reduce max or add more lines.");
        m.insert("botcreator_not_enough_usernames", "not enough usernames: {available} in file, need up to {needed} ({accounts}×{max}). Reduce max or add more lines.");
        m.insert("botcreator_too_many_warning", "⚠️ With minimum {min} × {accounts} acc = {total} bots. This may take a very long time.");
        m.insert("botcreator_starting", "{prefix} starting bot creation...");
        m.insert("botcreator_start_error", "{prefix} error sending /start to BotFather: {error}");
        m.insert("botcreator_resolve_flood", "{prefix} FLOOD_WAIT on resolve BotFather, waiting {seconds} sec...");
        m.insert("botcreator_resolve_flood_skip", "{prefix} FLOOD_WAIT {seconds} sec exceeds limit ({limit}), skipping account");
        m.insert("botcreator_names_exhausted", "{prefix} names exhausted");
        m.insert("botcreator_usernames_exhausted", "{prefix} usernames exhausted");
        m.insert("botcreator_creating", "{prefix} [{idx}/{total}] creating bot: {name} (@{username})");
        m.insert("botcreator_newbot_error", "{prefix} error sending /newbot: {error}");
        m.insert("botcreator_rate_limit_skip", "{prefix} BotFather rate limit {seconds} sec, skipping account");
        m.insert("botcreator_restricted", "{prefix} BotFather: account is restricted from creating bots, skipping");
        m.insert("botcreator_rate_limit_wait", "{prefix} rate limit, waiting {seconds} sec... (attempt {attempt}/3)");
        m.insert("botcreator_rate_limit_exhausted", "{prefix} /newbot attempts exhausted after rate limit");
        m.insert("botcreator_rate_limit_error", "{prefix} error: BotFather rate limit, skipping account");
        m.insert("botcreator_username_taken", "{prefix} username taken, trying: @{username}");
        m.insert("botcreator_created", "{prefix} bot created, token: {token_start}...{token_end}");
        m.insert("botcreator_token_error", "{prefix} [{idx}/{total}] error: could not get token. BotFather: {reason}");
        m.insert("botcreator_cleanup_done", "{prefix} BotFather blocked, history deleted");
        m.insert("botcreator_db_open_error", "open DB: {error}");
        m.insert("botcreator_db_tables_error", "create tables: {error}");
        m.insert("botcreator_newbot_exhausted", "/newbot attempts exhausted");
        m.insert("botcreator_no_botfather_reply", "no response from BotFather to /newbot");
        // Bot Parser
        m.insert("bot_parser_no_accounts", "No accounts selected");
        m.insert("bot_parser_task_name", "Bot parser: {count} acc.");
        m.insert("bot_parser_db_path", "Output DB: {path}");
        m.insert("bot_parser_account_error", "{prefix} account error: {error}");
        m.insert("bot_parser_collecting", "{prefix} collecting bot list...");
        m.insert("bot_parser_no_bots", "{prefix} account has no bots");
        m.insert("bot_parser_found", "{prefix} found bots: {count}");
        m.insert("bot_parser_revoke", "{prefix} [{idx}/{total}] regenerating @{username}");
        m.insert("bot_parser_token", "{prefix} [{idx}/{total}] getting token for @{username}");
        m.insert("bot_parser_bot_error", "{prefix} @{username}: {error}");
        m.insert("bot_parser_cleanup", "{prefix} BotFather blocked, history deleted");
        m.insert("bot_parser_result", "{prefix} done: bots {bots}, tokens {tokens}");
        m.insert("bot_parser_flood_wait", "{prefix} FLOOD_WAIT {seconds} sec exceeds limit ({limit}), skipping account");
        m.insert("bot_parser_flood_wait_wait", "{prefix} FLOOD_WAIT {seconds} sec, waiting...");
        m.insert("bot_parser_db_open_error", "open DB: {error}");
        m.insert("bot_parser_db_tables_error", "create tables: {error}");

        // Checker
        m.insert("checker_checking_accounts", "Checking {count} accounts...");
        m.insert("checker_no_accounts", "No accounts to check.");
        m.insert("checker_session_read_error", "ERROR reading .session: {error}");
        m.insert("checker_no_accounts_in_tdata", "No accounts in tdata: {path}");
        m.insert("checker_multi_account_tdata", "Multi-account tdata ({count} acc.): {path}");
        m.insert("checker_local_passcode", "Tdata protected by Local Passcode, skipped: {path}");
        m.insert("checker_parse_error", "PARSE ERROR: {error}");
        m.insert("checker_checking_progress", "[{idx}/{total}] Checking...");
        m.insert("checker_authkey_invalid_size", "  ERROR: auth_key has invalid size");
        m.insert("checker_connect_error", "  CONNECTION ERROR (5 attempts): {error}");
        m.insert("checker_invalid", "  INVALID: {error}");
        m.insert("checker_valid", "VALID id={id}");
        m.insert("checker_nft_tag", "    NFT tag: @{tag}");
        m.insert("checker_short_channel_tag", "    Short channel tag: @{username} ({title})");
        m.insert("checker_channel", "    Channel: {title} ({count} subs)");
        m.insert("checker_group", "    Group: {title} ({count} members)");
        m.insert("checker_channel_balance", "    {kind} \"{title}\": Stars={stars} TON={ton}");
        m.insert("checker_seed", "    Seed: {text}");
        m.insert("checker_downloaded", "    Downloaded: {path}");
        m.insert("checker_download_error", "    Download error {filename}: {error}");
        m.insert("checker_added_to_panel", "    Added to panel: {id}");
        m.insert("checker_role_premium", "Premium");
        m.insert("checker_role_default", "Checker");
        m.insert("checker_summary", "Done. Valid: {valid} | Invalid: {invalid}");

        // Stories extended
        m.insert("stories_read_error", "could not read {path}: {error}");
        m.insert("stories_read_media_error", "could not read media file: {error}");
        m.insert("stories_no_media", "no media files specified");
        m.insert("stories_read_tags_error", "could not read tags file: {error}");
        m.insert("stories_tags_empty", "usernames file is empty");
        m.insert("stories_task_name", "Stories: {count} acc.");
        m.insert("stories_done", "[{idx}/{total}] done: {msg}");
        m.insert("stories_error", "[{idx}/{total}] error: {error}");
        m.insert("stories_stopped", "stopped");
        m.insert("stories_caption_too_long", "caption too long, tags won't fit");
        m.insert("stories_caption_no_tag_fit", "caption too long, no tags will fit");
        m.insert("stories_caption_over_limit", "caption exceeds limit ({limit} chars)");
        m.insert("stories_premium_required", "account without Premium — stories unavailable");
        m.insert("stories_flood_wait", "FLOOD_WAIT {secs} sec (limit {limit})");
        m.insert("stories_uploaded", "{count} stories uploaded{tags}");
        m.insert("stories_tags_suffix", ", {count} tags");

        // Global Search extended
        m.insert("global_search_no_accounts", "no accounts selected");
        m.insert("global_search_task_name", "Global Search");
        m.insert("global_search_no_input_file", "no input file specified");
        m.insert("global_search_read_error", "could not read file: {error}");
        m.insert("global_search_skipped_invalid", "Skipped invalid words: {count}");
        m.insert("global_search_file_empty", "file is empty or contains no valid words");
        m.insert("global_search_loaded", "Loaded {words} words, {accounts} accounts, mode: {mode}, distribution: {distribution}, searchGlobal: {sg}");
        m.insert("global_search_open_file_error", "open file: {error}");
        m.insert("global_search_results_db", "Results DB: {path}");
        m.insert("global_search_thread_connect_error", "Thread {idx}: could not connect: {error}");
        m.insert("global_search_thread_no_words", "Thread {idx}: no words to process");
        m.insert("global_search_thread_words", "Thread {idx}: received {count} words");
        m.insert("global_search_word_result", "[{idx}/{total}] \"{word}\" — found {found} (new: {new})");
        m.insert("global_search_result", "Done: unique found={count}, file: {path}");
        m.insert("global_search_mode_channels", "channels");
        m.insert("global_search_mode_groups", "groups");
        m.insert("global_search_mode_users", "users");
        m.insert("global_search_mode_all", "all");
        m.insert("global_search_yes", "yes");
        m.insert("global_search_no", "no");

        // Link Checker
        m.insert("link_checker_no_accounts", "no accounts selected");
        m.insert("link_checker_task_name", "Link Validation");
        m.insert("link_checker_no_input_file", "no input file specified");
        m.insert("link_checker_read_error", "could not read file: {error}");
        m.insert("link_checker_file_empty", "file is empty or contains no links");
        m.insert("link_checker_loaded", "Loaded {links} links, {accounts} accounts (threads)");
        m.insert("link_checker_no_output_file", "no output file specified");
        m.insert("link_checker_db_open_error", "open DB: {error}");
        m.insert("link_checker_db_tables_error", "create tables: {error}");
        m.insert("link_checker_thread_connect_error", "Thread {idx}: could not connect account: {error}");
        m.insert("link_checker_valid", "[{idx}/{total}] {link} — valid ({kind}: {name})");
        m.insert("link_checker_invalid", "[{idx}/{total}] {link} — invalid");
        m.insert("link_checker_skipped", "[{idx}/{total}] {link} — skipped: {reason}");
        m.insert("link_checker_result", "Done: valid={valid}, invalid={invalid}, skipped={skipped}, DB: {path}");
        m.insert("link_checker_retry_limit", "retry limit exceeded");

        // Cloner extended
        m.insert("cloner_task_name", "Cloner: {id}");
        m.insert("cloner_source", "Source: {title} (id={id})");
        m.insert("cloner_destination", "Destination: id={id}");
        m.insert("cloner_sweep_error", "Could not clean up service messages: {error}");
        m.insert("cloner_left_source", "Left source channel");
        m.insert("cloner_leave_source_error", "Could not leave source: {error}");
        m.insert("cloner_stats", "Done: copied={copied} skipped={skipped} errors={errors}");
        m.insert("cloner_from_gt_to", "from_id is greater than to_id");
        m.insert("cloner_no_messages", "No messages in specified range");
        m.insert("cloner_collected", "Collected posts: {count} (id {lo}..={hi})");
        m.insert("cloner_skipped_service", "skipped: msg={id} (service)");
        m.insert("cloner_skipped_reason", "skipped: msg={id} ({reason})");
        m.insert("cloner_skipped_media", "skipped: msg={id} ({reason})");
        m.insert("cloner_skipped_size", "skipped: msg={id} ({reason}, {kb} KB)");
        m.insert("cloner_error_msg", "error: msg={id} {error}");
        m.insert("cloner_copied_msg", "copied: msg={id} -> dst={dst}");
        m.insert("cloner_sweep_count", "Cleaned up service messages: {count}");
        m.insert("cloner_sweep_result", "Service message cleanup returned: {error}");

        // Inviter extra (remaining hardcoded)
        m.insert("inviter_no_target_groups_err", "no target groups");
        m.insert("inviter_target_channel_info", "target channel: id={id} title=\"{title}\"");
        m.insert("inviter_unknown_uid_skip", "unknown user_id for {id}, skipping");
        m.insert("inviter_admin_granted_msg", "admin granted user_id={uid}");
        m.insert("inviter_admin_error_msg", "admin error user_id={uid}: {error}");
        m.insert("inviter_no_uid_err", "no user_id");
        m.insert("inviter_revoke_nobody_msg", "nobody to revoke");
        m.insert("inviter_main_uid_error_msg", "failed to get main account user_id: {error}");
        m.insert("inviter_revoking_msg", "revoking admin rights...");
        m.insert("inviter_revoke_error_msg", "revoke error user_id={uid}: {error}");
        m.insert("inviter_revoked_msg", "revoked admin rights: {done}/{total}");

        // Username Checker extended
        m.insert("uchecker_task_name", "Username Checker");
        m.insert("uchecker_no_input_file", "no input file specified");
        m.insert("uchecker_read_file_error", "could not read file: {error}");
        m.insert("uchecker_invalid_short", "@{name} — tag invalid (less than 4 chars), skipping");
        m.insert("uchecker_invalid_chars", "@{name} — tag invalid (disallowed chars), skipping");
        m.insert("uchecker_skipped_invalid", "Skipped invalid/duplicates: {count}");
        m.insert("uchecker_file_empty", "file is empty or contains no valid usernames");
        m.insert("uchecker_no_proxies", "no proxies — username checker requires proxies");
        m.insert("uchecker_loaded", "Loaded {usernames} usernames, {proxies} proxies, {threads} threads");
        m.insert("uchecker_open_db_error", "open DB: {error}");
        m.insert("uchecker_create_table_error", "create table: {error}");
        m.insert("uchecker_error_user", "[{idx}/{total}] @{name} error: {error}");
        m.insert("uchecker_progress", "[{idx}/{total}] @{name} — {status}");
        m.insert("uchecker_status_free", "free");
        m.insert("uchecker_status_taken", "taken");
        m.insert("uchecker_status_for_sale", "for sale on Fragment");
        m.insert("uchecker_status_sold", "sold");
        m.insert("uchecker_status_error", "error");
        m.insert("uchecker_autoclaim", "Auto-claim: {free} free tags, {accounts} accounts");
        m.insert("uchecker_claimed", "  @{name} — claimed on account {phone}");
        m.insert("uchecker_claim_failed", "  @{name} — could not claim: {error}");
        m.insert("uchecker_result", "Done: free={free}, taken={taken}, file: {path}");

        // Masslooking extended
        m.insert("masslooking_no_accounts", "no accounts selected");
        m.insert("masslooking_task_name", "Masslooking: {count} acc.");
        m.insert("masslooking_targets_count", "targets: {count}");
        m.insert("masslooking_stories_error", "stories error user={user_id}: {error}");
        m.insert("masslooking_stories_viewed", "viewed {count} stories user={user_id}");
        m.insert("masslooking_reaction_sent", "reaction {emoji} on story user={user_id}");
        m.insert("masslooking_reaction_error", "reaction error user={user_id}: {error}");
        m.insert("masslooking_reply_sent", "reply sent user={user_id}");
        m.insert("masslooking_reply_error", "reply error user={user_id}: {error}");
        m.insert("masslooking_processed", "total processed: {count}");
        m.insert("masslooking_inbox_found", "found {count} users from inbox");
        m.insert("masslooking_chat_found", "found {count} users from chat");

        // Actions extended (remaining hardcoded)
        m.insert("actions_error_generic", "ERROR: {error}");
        m.insert("actions_prefix_too_long", "username_prefix too long: {chars} chars (max 30)");

        // Cloner transform (skip reasons)
        m.insert("cloner_skip_keyword", "contains stop word");
        m.insert("cloner_skip_documents", "documents disabled");
        m.insert("cloner_skip_photos", "photos disabled");
        m.insert("cloner_skip_videos", "videos disabled");
        m.insert("cloner_skip_video_msg", "video messages disabled");
        m.insert("cloner_skip_ext_link", "external links disabled");
        m.insert("cloner_skip_tg_link", "telegram links disabled");
        m.insert("cloner_skip_file_size", "file exceeds size limit");
        m.insert("cloner_skip_video_size", "video exceeds size limit");
        m.insert("cloner_skip_photo_size", "photo exceeds size limit");

        // Converter pyro
        m.insert("converter_pyro_open_error", "Could not open Pyrogram session: {error}");
        m.insert("converter_pyro_table_error", "Pyrogram: sessions table corrupted: {error}");
        m.insert("converter_pyro_empty_table", "Pyrogram: empty or corrupted sessions table: {error}");
        m.insert("converter_pyro_authkey_size", "Pyrogram: auth_key has invalid size ({bytes} bytes)");
        m.insert("converter_pyro_create_error", "Could not create Pyrogram session: {error}");
        m.insert("converter_pyro_schema_error", "Pyrogram: schema creation error: {error}");
        m.insert("converter_pyro_write_error", "Pyrogram: sessions write error: {error}");
        m.insert("converter_pyro_version_error", "Pyrogram: version write error: {error}");

        // Converter telethon
        m.insert("converter_telethon_open_session_error", "Could not open session file. It may be corrupted or have an unknown format. ({error})");
        m.insert("converter_telethon_table_error", "Session file corrupted: sessions table not found. ({error})");
        m.insert("converter_telethon_empty_error", "Session file is empty or corrupted. ({error})");
        m.insert("converter_telethon_create_error", "Could not create session file: {error}");
        m.insert("converter_telethon_schema_error", "Schema creation error: {error}");
        m.insert("converter_telethon_version_error", "Version write error: {error}");
        m.insert("converter_telethon_write_error", "Session write error: {error}");

        // Queue validate
        m.insert("validate_task_name", "Validating {count} accounts");
        m.insert("validate_checking_attempt", "Checking (attempt {attempt}/5)");
        m.insert("validate_restrictions", "Checking restrictions...");
        m.insert("validate_2fa_unknown", "Unknown");
        m.insert("validate_2fa_hint", "Unknown, hint: {hint}");

        // MTProto invite
        m.insert("invite_empty_link", "empty link");
        m.insert("invite_addlist_not_channel", "addlist link is not a channel or group");
        m.insert("invite_parse_error", "could not parse link");
        m.insert("invite_already_member_no_hash", "already a member but response has no access_hash — try a channel link");
        m.insert("invite_request_needed", "channel \"{label}\" requires admin approval (join request). Submit request and wait for manual approval.");
        m.insert("invite_request_sent", "channel requires admin approval — request sent. Cannot proceed without approval.");
        m.insert("invite_already_no_hash", "already a member but access_hash not obtained");

        // MTProto client
        m.insert("mtproto_reconnecting", "reconnecting...");
        m.insert("mtproto_network_error", "network error (attempt {attempt}/5): {error}, reconnecting in {delay}ms...");
        m.insert("mtproto_flood_over_limit", "FLOOD_WAIT: {wait} sec > limit {limit} sec, aborting");
        m.insert("mtproto_flood_waiting", "FLOOD_WAIT: waiting {wait} sec...");

        // Account session
        m.insert("session_read_json_error", "Could not read .json file: {error}");
        m.insert("session_parse_json_error", "JSON file parse error: {error}");
        m.insert("session_serialize_error", "Serialization error: {error}");
        m.insert("session_write_json_error", "Could not write .json file: {error}");

        // Accounts reauth
        m.insert("reauth_status", "Re-authorizing...");
        m.insert("reauth_step", "Re-authorizing ({step}/3)");
        m.insert("reauth_signing_in", "Re-authorizing (signing in)");

        // Accounts auth_login
        m.insert("auth_connect_error", "Could not connect to Telegram servers. {error}");
        m.insert("auth_dh_error", "DH key exchange error: {error}");
        m.insert("auth_2fa_not_set", "Two-factor authentication is not set");
        m.insert("auth_srp_error", "SRP computation error: {error}");
        m.insert("auth_session_expired", "Session expired or not found");
        m.insert("auth_session_write_error", "Session write error: {error}");

        // Cloner destination
        m.insert("cloner_dest_parse_error", "could not parse destination ID/link");
        m.insert("cloner_dest_numeric_unsupported", "provide @username or channel link; plain ID without access_hash is not supported");
        m.insert("cloner_dest_username_taken", "username @{username} is taken or invalid");
        m.insert("cloner_dest_avatar_copied", "Avatar copied");
        m.insert("cloner_dest_avatar_error", "Avatar copy error: {error}");
        m.insert("cloner_dest_no_avatar", "Source has no extractable avatar — skipping copy");

        // Cloner config
        m.insert("cloner_cfg_public_no_username", "public channel requires a username");
        m.insert("cloner_cfg_no_existing_id", "no existing channel ID specified");

        // Proxy
        m.insert("proxy_no_available", "no available proxies, add proxies or enable no-proxy mode in settings");
        m.insert("proxy_connect_error", "Could not connect to Telegram servers. Check proxy validity. ({error})");
        m.insert("proxy_validate_task", "Validating {count} proxies");

        // LLM
        m.insert("llm_not_configured", "LLM not configured (check Settings -> LLM)");
        m.insert("llm_specify_url_token", "specify API URL and token");

        // Accounts connect
        m.insert("connect_session_error", "session: {error}");
        m.insert("connect_invalid_authkey", "invalid auth_key");

        // Checker validate
        m.insert("checker_validate_authkey_size", "auth_key has invalid size");
        m.insert("checker_validate_connect", "Connection: {error}");

        // Quick Actions
        m.insert("quick_mailing", "New mailing");
        m.insert("quick_checker", "Check accounts");
        m.insert("quick_inviter", "Invite to group");
        m.insert("quick_parser", "Parse members");

        // Checker task
        m.insert("checker_task_name", "Checker: {count} accounts");

        // Inviter DB
        m.insert("inviter_db_open_error", "open users DB: {error}");
        m.insert("inviter_db_create_tables", "create users tables: {error}");
        m.insert("inviter_stats_db_open_error", "open stats DB: {error}");
        m.insert("inviter_stats_db_create_tables", "create stats tables: {error}");

        // First Comment LLM
        m.insert("first_comment_post_prefix", "Post: {text}");

        // Converter tdata
        m.insert("converter_tdata_no_userid", "user_id=0 — cannot create valid TData without user_id");

        // MTProto transport
        m.insert("mtproto_connect_error", "Could not connect to Telegram servers. Check proxy validity. ({error})");

        // Account actions
        m.insert("actions_2fa_already_set_err", "2FA already set");
        m.insert("actions_2fa_no_algo_params", "2FA: server did not return algorithm parameters");
        m.insert("actions_2fa_srp_stale", "2FA: SRP parameters stale, skipping");
        m.insert("actions_response_too_small", "response too small, probably not an image");
        m.insert("actions_db_open_error", "open actions DB: {error}");

        // Two-FA display values
        m.insert("two_fa_unknown", "Unknown");
        m.insert("two_fa_unknown_set", "Set, unknown");
        m.insert("two_fa_unknown_hint", "Unknown, hint: {hint}");

        // Role display values
        m.insert("role_premium", "Premium");
        m.insert("role_checker", "Checker");

        m
    };
}
