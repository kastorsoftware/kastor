import { useState, useEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Wifi, Plus, Trash2, X, ChevronDown, ShieldCheck,
  ShieldAlert, Clock, Search, FileText,
} from "lucide-react";
import { ThreadInput } from "@/components/ThreadInput";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { useT } from "@/i18n";

interface ProxyData {
  id: string;
  proxy_type: "Socks5" | "Socks4" | "Https";
  host: string;
  port: number;
  username: string | null;
  password: string | null;
  status: "Valid" | "Invalid" | "Unchecked";
  last_check: string | null;
}

export function ProxySettings() {
  const t = useT();
  const [proxies, setProxies] = useState<ProxyData[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [validating, setValidating] = useState(false);
  const [addType, setAddType] = useState<"socks5" | "socks4" | "https">("socks5");
  const [typeDropdownOpen, setTypeDropdownOpen] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<"selected" | "all" | null>(null);
  const [search, setSearch] = useState("");
  const [threads, setThreads] = useState(10);

  // load saved proxy thread count
  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ proxy_threads?: number }>("get_settings").then((s) => {
      if (s.proxy_threads) setThreads(s.proxy_threads);
    }).catch(() => {});
  }, []);
  const [page, setPage] = useState(0);
  const pageSize = 30;
  const [dupeWarning, setDupeWarning] = useState<number | null>(null);
  const [proxyResult, setProxyResult] = useState<{ valid: number; invalid: number } | null>(null);
  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  const refresh = async () => {
    if (IS_DEV) return;
    const list = await invoke<ProxyData[]>("get_proxies");
    setProxies(list);
  };

  useEffect(() => { refresh(); }, []);

  // background refresh every 3 seconds
  useEffect(() => {
    if (IS_DEV) return;
    const interval = setInterval(refresh, 3000);
    return () => clearInterval(interval);
  }, []);

  const filtered = useMemo(() => proxies.filter((p) => {
    if (!search) return true;
    const s = search.toLowerCase();
    return p.host.toLowerCase().includes(s) || String(p.port).includes(s) ||
      (p.username && p.username.toLowerCase().includes(s));
  }), [proxies, search]);

  const allSelected = filtered.length > 0 && filtered.every((p) => selected.has(p.id));
  const hasSelection = selected.size > 0;

  const toggleAll = () => {
    if (allSelected) setSelected(new Set());
    else setSelected(new Set(filtered.map((p) => p.id)));
  };

  const toggleOne = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const handleValidate = async () => {
    if (IS_DEV) return;
    setValidating(true);
    const ids = hasSelection ? Array.from(selected) : [];
    await invoke<string>("enqueue_validate_proxies", { ids, threads });
    setTimeout(async () => {
      const poll = setInterval(async () => {
        const [queued, running] = await invoke<[number, number]>("get_queue_stats");
        if (queued === 0 && running === 0) {
          clearInterval(poll);
          const freshList = await invoke<ProxyData[]>("get_proxies");
          setProxies(freshList);
          setValidating(false);
          setSelected(new Set());
          const valid = freshList.filter((p) => p.status === "Valid").length;
          const invalid = freshList.filter((p) => p.status === "Invalid").length;
          setProxyResult({ valid, invalid });
        } else {
          await refresh();
        }
      }, 1500);
    }, 500);
  };

  const handleDelete = async () => {
    if (IS_DEV) return;
    if (deleteConfirm === "selected") {
      await invoke("remove_proxies", { ids: Array.from(selected) });
    } else if (deleteConfirm === "all") {
      await invoke("clear_proxies");
    }
    setDeleteConfirm(null);
    setSelected(new Set());
    await refresh();
  };

  const handleAddFromTxt = async () => {
    if (IS_DEV) return;
    const path = await invoke<string>("get_proxy_txt_path", { proxyType: addType });
    await invoke("open_file_in_editor", { path });
  };

  const handleReloadFromTxt = async () => {
    if (IS_DEV) return;
    const [count, dupes] = await invoke<[number, number]>("import_proxies_from_txt", { proxyType: addType });
    if (dupes > 0) {
      setDupeWarning(dupes);
    }
    if (count > 0) await refresh();
  };

  const [removeOneId, setRemoveOneId] = useState<string | null>(null);

  const handleRemoveOne = (id: string) => {
    setRemoveOneId(id);
  };

  const confirmRemoveOne = async () => {
    if (IS_DEV || !removeOneId) return;
    await invoke("remove_proxies", { ids: [removeOneId] });
    setRemoveOneId(null);
    await refresh();
  };

  const typeLabelFn = (tp: string) => {
    switch (tp) { case "Socks5": return "SOCKS5"; case "Socks4": return "SOCKS4"; case "Https": return "HTTPS"; default: return tp; }
  };

  const typeColor = (tp: string) => {
    switch (tp) { case "Socks5": return "oklch(0.65 0.1 280)"; case "Socks4": return "oklch(0.65 0.1 60)"; case "Https": return "oklch(0.65 0.1 150)"; default: return "oklch(0.6 0.05 300)"; }
  };

  const statusIcon = (s: string) => {
    switch (s) {
      case "Valid": return <ShieldCheck className="h-3.5 w-3.5 text-[oklch(0.65_0.1_150)]" />;
      case "Invalid": return <ShieldAlert className="h-3.5 w-3.5 text-[oklch(0.55_0.1_25)]" />;
      case "Checking": return <Clock className="h-3.5 w-3.5 text-[oklch(0.65_0.1_280)] animate-pulse" />;
      default: return <Clock className="h-3.5 w-3.5 text-muted-foreground" />;
    }
  };

  const statusLabel = (s: string) => {
    switch (s) { case "Valid": return t("proxy.statusValid"); case "Invalid": return t("proxy.statusInvalid"); case "Checking": return t("proxy.statusChecking"); default: return t("proxy.statusUnchecked"); }
  };

  const formatLastCheck = (ts: string | null) => {
    if (!ts) return "—";
    const secs = parseInt(ts);
    if (isNaN(secs)) return ts;
    const d = new Date(secs * 1000);
    return d.toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
  };

  const maskPassword = (p: string | null) => p ? "****" : "";

  return (
    <div className="space-y-6">
      {/* action bar */}
      <div className="flex items-center gap-3 flex-wrap">
        <button
          onClick={handleAddFromTxt}
          className="flex h-8 w-8 items-center justify-center rounded-md border border-border bg-card text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
          title={t("proxy.addFromTxt")}
        >
          <Plus className="h-4 w-4" />
        </button>

        <div className="relative" ref={useRef<HTMLDivElement>(null)}>
          <button
            onClick={() => setTypeDropdownOpen(!typeDropdownOpen)}
            className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
          >
            {addType.toUpperCase()}
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          </button>
          {typeDropdownOpen && (
            <div className="absolute top-full left-0 mt-1 z-50 min-w-[100px] rounded-md border border-border bg-card shadow-lg py-1">
              {(["socks5", "socks4", "https"] as const).map((tp) => (
                <button
                  key={tp}
                  onClick={() => { setAddType(tp); setTypeDropdownOpen(false); }}
                  className={`w-full text-left px-3 py-1.5 text-sm hover:bg-accent/50 ${addType === tp ? "text-primary font-medium" : ""}`}
                >
                  {tp.toUpperCase()}
                </button>
              ))}
            </div>
          )}
        </div>

        <button
          onClick={handleReloadFromTxt}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
          title={t("proxy.reload")}
        >
          <FileText className="h-3.5 w-3.5" />
          {t("proxy.reload")}
        </button>

        <button
          onClick={handleValidate}
          disabled={validating}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition disabled:opacity-50"
        >
          <ShieldCheck className="h-3.5 w-3.5" />
          {validating ? t("proxy.checking") : hasSelection ? t("proxy.validateSelected") : t("proxy.validateAll")}
        </button>

        <div className="flex items-center gap-1.5">
          <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
          <ThreadInput value={threads} onChange={(v) => {
            setThreads(v);
            if (!IS_DEV) invoke("patch_settings", { patch: { proxy_threads: v } }).catch(() => {});
          }} min={1} max={1000} />
        </div>

        <button
          onClick={() => setDeleteConfirm(hasSelection ? "selected" : "all")}
          className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-destructive hover:border-destructive/30 transition"
        >
          <Trash2 className="h-3.5 w-3.5" />
          {hasSelection ? t("proxy.deleteSelected", { count: selected.size }) : t("proxy.deleteAll")}
        </button>

        <div className="ml-auto relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("common.search")}
            className="rounded-md border border-border bg-card pl-9 pr-3 py-1.5 text-sm outline-none focus:border-primary/50 w-48"
          />
        </div>
      </div>

      <div className="text-sm font-semibold text-muted-foreground">
        {t("proxy.total")}: {proxies.length} | {t("proxy.statusValid")}: {proxies.filter(p => p.status === "Valid").length} | {t("proxy.statusInvalid")}: {proxies.filter(p => p.status === "Invalid").length}
      </div>

      {proxies.length > 0 ? (
        <div className="rounded-md border border-border">
          <div className="max-h-[60vh] overflow-y-auto scrollbar-thin">
          <table className="w-full text-sm">
            <thead className="bg-card border-b border-border sticky top-0 z-10">
              <tr>
                <th className="w-10 px-3 py-2.5">
                  <input type="checkbox" checked={allSelected} onChange={toggleAll} className="rounded border-border" />
                </th>
                <th className="w-10 px-2 py-2.5 text-left text-muted-foreground font-medium">#</th>
                <th className="px-3 py-2.5 text-left text-muted-foreground font-medium">{t("proxy.type")}</th>
                <th className="px-3 py-2.5 text-left text-muted-foreground font-medium">{t("proxy.address")}</th>
                <th className="px-3 py-2.5 text-left text-muted-foreground font-medium">{t("proxy.status")}</th>
                <th className="px-3 py-2.5 text-left text-muted-foreground font-medium">{t("proxy.lastCheck")}</th>
                <th className="w-10 px-3 py-2.5"></th>
              </tr>
            </thead>
            <tbody>
              {filtered.slice(page * pageSize, (page + 1) * pageSize).map((px) => {
                const color = typeColor(px.proxy_type);
                const addrStr = px.username
                  ? `${px.host}:${px.port}:${px.username}:${maskPassword(px.password)}`
                  : `${px.host}:${px.port}`;
                return (
                  <tr key={px.id} className={`border-b border-border transition ${selected.has(px.id) ? "bg-primary/5" : "hover:bg-card/60"}`}>
                    <td className="px-3 py-2.5">
                      <input type="checkbox" checked={selected.has(px.id)} onChange={() => toggleOne(px.id)} className="rounded border-border" />
                    </td>
                    <td className="px-2 py-2.5 text-muted-foreground text-xs">
                      {proxies.indexOf(px) + 1}
                    </td>
                    <td className="px-3 py-2.5">
                      <span className="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium"
                        style={{ color, borderColor: `color-mix(in oklch, ${color} 30%, transparent)`, background: `color-mix(in oklch, ${color} 6%, transparent)` }}>
                        {typeLabelFn(px.proxy_type)}
                      </span>
                    </td>
                    <td className="px-3 py-2.5 font-mono text-[11px] font-semibold">{addrStr}</td>
                    <td className="px-3 py-2.5">
                      <span className="inline-flex items-center gap-1.5 text-xs font-semibold">
                        {statusIcon(px.status)}
                        {statusLabel(px.status)}
                      </span>
                    </td>
                    <td className="px-3 py-2.5 text-xs text-muted-foreground">{formatLastCheck(px.last_check)}</td>
                    <td className="px-3 py-2.5">
                      <button onClick={() => handleRemoveOne(px.id)} className="rounded-md p-1 text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition opacity-40 hover:opacity-100">
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          </div>
        </div>
      ) : (
        <div className="flex flex-col items-center justify-center py-16 rounded-md border border-dashed border-border text-muted-foreground">
          <Wifi className="h-10 w-10 mb-3 opacity-40" />
          <div className="text-sm">{t("proxy.noProxies")}</div>
          <div className="text-xs mt-1 opacity-60">{t("proxy.noProxiesHint")}</div>
        </div>
      )}

      <div className="flex items-center justify-center gap-4">
          <button
            onClick={() => setPage((p) => Math.max(0, p - 1))}
            disabled={page === 0}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm disabled:opacity-30 hover:border-primary/50 transition"
          >
            {t("common.back")}
          </button>
          <span className="text-sm text-muted-foreground whitespace-nowrap">
            {filtered.length > pageSize
              ? `${page * pageSize + 1}–${Math.min((page + 1) * pageSize, filtered.length)} / ${filtered.length}`
              : `${filtered.length}`}
          </span>
          <button
            onClick={() => setPage((p) => Math.min(Math.ceil(filtered.length / pageSize) - 1, p + 1))}
            disabled={(page + 1) * pageSize >= filtered.length}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm disabled:opacity-30 hover:border-primary/50 transition"
          >
            {t("common.forward")}
          </button>
      </div>

      <AlertDialog open={deleteConfirm !== null} onOpenChange={(o) => !o && setDeleteConfirm(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("proxy.deleteConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {deleteConfirm === "selected"
                ? t("proxy.deleteConfirmSelected", { count: selected.size })
                : t("proxy.deleteConfirmAll", { count: proxies.length })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={handleDelete}>{t("common.delete")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {removeOneId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-80 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("proxy.deleteConfirmTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("proxy.deleteConfirmSelected", { count: 1 })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setRemoveOneId(null)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={confirmRemoveOne}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm text-muted-foreground font-medium hover:text-destructive hover:border-destructive/30 transition"
              >
                {t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}

      {dupeWarning !== null && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <p className="text-sm text-muted-foreground mb-4">{t("proxy.duplicatesSkipped", { count: dupeWarning })}</p>
            <button
              onClick={() => setDupeWarning(null)}
              className="w-full rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
            >
              {t("common.ok")}
            </button>
          </div>
        </div>
      )}

      {proxyResult && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-3">{t("proxy.resultTitle")}</h3>
            <div className="text-sm space-y-2 mb-4">
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("proxy.resultValid")}:</span>
                <span className="font-medium text-[oklch(0.65_0.1_150)]">{proxyResult.valid}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("proxy.resultInvalid")}:</span>
                <span className="font-medium text-[oklch(0.55_0.1_25)]">{proxyResult.invalid}</span>
              </div>
            </div>
            <div className="flex flex-col gap-2">
              <button
                onClick={() => setProxyResult(null)}
                className="w-full rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
              {proxyResult.invalid > 0 && (
                <button
                  onClick={async () => {
                    if (!IS_DEV) {
                      const invalidIds = proxies.filter((p) => p.status === "Invalid").map((p) => p.id);
                      await invoke("remove_proxies", { ids: invalidIds });
                      await refresh();
                    }
                    setProxyResult(null);
                  }}
                  className="w-full rounded-md border border-border px-3 py-2 text-sm text-muted-foreground font-medium hover:text-destructive hover:border-destructive/30 transition"
                >
                  {t("proxy.deleteInvalid")}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
