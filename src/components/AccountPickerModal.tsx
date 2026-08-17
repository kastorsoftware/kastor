import { useState, useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, ChevronDown, Tag, Filter } from "lucide-react";
import { useT } from "@/i18n";
import {
  AFRICA_COUNTRIES,
  AMERICAS_COUNTRIES,
  ASIA_COUNTRIES,
  CIS_COUNTRIES,
  EUROPE_COUNTRIES,
  MIDDLE_EAST_COUNTRIES,
  buildGeoOptions,
  getCountryName,
  getGeoLabel,
  getGeoSearchText,
  type GeoOption,
} from "@/lib/countryData";

interface AccountData {
  id: string;
  phone: string;
  geo: string;
  status: string;
  aging: string;
  role: string;
  name: string;
  username: string;
  two_fa: string;
  premium: string;
  user_id: number;
}

interface AccountPickerModalProps {
  open: boolean;
  onClose: () => void;
  onSelect: (ids: string[]) => void;
  title?: string;
}

type AgingFilter = "all" | "day" | "week" | "month" | "year" | "more";

const agingLabels: Record<AgingFilter, string> = {
  all: "all",
  day: "day",
  week: "week",
  month: "month",
  year: "year",
  more: "more",
};

function matchGeo(geo: string, filter: string): boolean {
  if (filter === "all") return true;
  if (filter === "cis") return CIS_COUNTRIES.includes(geo);
  if (filter === "non-cis") return !CIS_COUNTRIES.includes(geo);
  if (filter === "asia") return ASIA_COUNTRIES.includes(geo);
  if (filter === "europe") return EUROPE_COUNTRIES.includes(geo);
  if (filter === "americas") return AMERICAS_COUNTRIES.includes(geo);
  if (filter === "africa") return AFRICA_COUNTRIES.includes(geo);
  if (filter === "middle-east") return MIDDLE_EAST_COUNTRIES.includes(geo);
  return geo === filter;
}

export function AccountPickerModal({ open, onClose, onSelect, title }: AccountPickerModalProps) {
  const t = useT();
  const geoOptions = useMemo(() => buildGeoOptions(), [t]);
  const [accounts, setAccounts] = useState<AccountData[]>([]);
  const [roles, setRoles] = useState<string[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const [roleFilter, setRoleFilter] = useState("all");
  const [geoFilter, setGeoFilter] = useState("all");
  const [agingFilter, setAgingFilter] = useState<AgingFilter>("all");
  const [openFilter, setOpenFilter] = useState<string | null>(null);
  // Advanced filters
  const [showFilters, setShowFilters] = useState(false);
  const [filterPremium, setFilterPremium] = useState<"all" | "yes" | "no">("all");
  const [filterTwoFa, setFilterTwoFa] = useState<"all" | "yes" | "no">("all");
  const [filterStatus, setFilterStatus] = useState<Set<string>>(new Set());
  const [filterHasUsername, setFilterHasUsername] = useState<"all" | "yes" | "no">("all");
  const [filterIdMin, setFilterIdMin] = useState("");
  const [filterIdMax, setFilterIdMax] = useState("");
  // copy feedback uses one slot keyed by "<accId>:<field>"
  const [copiedCellKey, setCopiedCellKey] = useState<string | null>(null);

  const copyCell = async (accId: string, field: string, text: string) => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand("copy"); } catch {}
      document.body.removeChild(ta);
    }
    const key = `${accId}:${field}`;
    setCopiedCellKey(key);
    setTimeout(() => {
      setCopiedCellKey((prev) => (prev === key ? null : prev));
    }, 1200);
  };

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (!open) return;
    if (IS_DEV) { setAccounts([]); setRoles([]); return; }
    invoke<{ accounts: AccountData[] }>("get_accounts_with_stats").then((d) => setAccounts(d.accounts));
    invoke<string[]>("get_roles").then(setRoles);
    setSelected(new Set());
    setSearch("");
    setRoleFilter("all");
    setGeoFilter("all");
    setAgingFilter("all");
    setShowFilters(false);
    setFilterPremium("all");
    setFilterTwoFa("all");
    setFilterStatus(new Set());
    setFilterHasUsername("all");
    setFilterIdMin("");
    setFilterIdMax("");
  }, [open]);

  const activeFilterCount = [
    roleFilter !== "all",
    geoFilter !== "all",
    agingFilter !== "all",
    filterPremium !== "all",
    filterTwoFa !== "all",
    filterStatus.size > 0,
    filterHasUsername !== "all",
    !!filterIdMin,
    !!filterIdMax,
  ].filter(Boolean).length;

  const resetAdvancedFilters = () => {
    setRoleFilter("all");
    setGeoFilter("all");
    setAgingFilter("all");
    setFilterPremium("all");
    setFilterTwoFa("all");
    setFilterStatus(new Set());
    setFilterHasUsername("all");
    setFilterIdMin("");
    setFilterIdMax("");
  };

  const filtered = useMemo(() => {
    return accounts.filter((a) => {
      if (roleFilter !== "all" && a.role !== roleFilter) return false;
      if (!matchGeo(a.geo, geoFilter)) return false;
      if (agingFilter !== "all" && !matchAging(a.aging, agingFilter)) return false;
      if (search) {
        const s = search.toLowerCase().replace(/^\+/, "");
        if (!a.phone.toLowerCase().includes(s) && !a.name.toLowerCase().includes(s) && !a.username.toLowerCase().includes(s)) return false;
      }
      // Premium filter
      if (filterPremium === "yes" && !a.premium) return false;
      if (filterPremium === "no" && a.premium) return false;
      // 2FA filter
      if (filterTwoFa === "yes" && !a.two_fa) return false;
      if (filterTwoFa === "no" && a.two_fa) return false;
      // Username filter
      if (filterHasUsername === "yes" && !a.username) return false;
      if (filterHasUsername === "no" && a.username) return false;
      // Status filter (multi-select)
      if (filterStatus.size > 0) {
        const s = a.status;
        const matchesAny = Array.from(filterStatus).some((f) => {
          if (f === "clean") return s === "Без ограничений" || s === "No restrictions";
          if (f === "spamblock") return s.includes("спамблок") || s.includes("Спамблок") || s.includes("spamblock") || s.includes("Spamblock");
          if (f === "frozen") return s === "Заморожен" || s === "Frozen";
          if (f === "invalid") return s === "Невалид" || s === "Invalid";
          if (f === "unchecked") return s === "Не проверен" || s === "Unchecked";
          return false;
        });
        if (!matchesAny) return false;
      }
      // ID digit count filter (Telegram user_id)
      if (filterIdMin) {
        const minDigits = parseInt(filterIdMin, 10);
        if (!isNaN(minDigits) && a.user_id > 0) {
          const digits = a.user_id.toString().length;
          if (digits < minDigits) return false;
        }
      }
      if (filterIdMax) {
        const maxDigits = parseInt(filterIdMax, 10);
        if (!isNaN(maxDigits) && a.user_id > 0) {
          const digits = a.user_id.toString().length;
          if (digits > maxDigits) return false;
        }
      }
      return true;
    });
  }, [accounts, roleFilter, geoFilter, agingFilter, search, filterPremium, filterTwoFa, filterStatus, filterHasUsername, filterIdMin, filterIdMax]);

  const [page, setPage] = useState(0);
  const pageSize = 50;

  useEffect(() => { setPage(0); }, [roleFilter, geoFilter, agingFilter, search, filterPremium, filterTwoFa, filterStatus, filterHasUsername, filterIdMin, filterIdMax]);

  const allSelected = filtered.length > 0 && filtered.every((a) => selected.has(a.id));

  const toggleAll = () => {
    if (allSelected) setSelected(new Set());
    else setSelected(new Set(filtered.map((a) => a.id)));
  };

  const toggleOne = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="rounded-xl border border-border bg-card w-[1080px] max-h-[80vh] shadow-2xl flex flex-col" onClick={(e) => e.stopPropagation()}>
        {/* header */}
        <div className="px-6 pt-5 pb-3 border-b border-border">
          <h3 className="text-lg font-semibold">{title || t("accountPicker.title")}</h3>
          <div className="flex items-center gap-3 mt-3">
            <button
              onClick={() => setShowFilters(!showFilters)}
              className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
            >
              <Filter className="h-3 w-3" />
              {t("filters.title")}
              <ChevronDown className={`h-3 w-3 transition-transform ${showFilters ? "" : "-rotate-90"}`} />
              {activeFilterCount > 0 && (
                <span className="ml-1 rounded-full bg-primary/20 text-primary text-[10px] px-1.5 py-0.5 font-semibold">{activeFilterCount}</span>
              )}
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
          {/* Expanded filters panel */}
          {showFilters && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <PickerFilter
                id="role"
                label={t("accountPicker.role")}
                value={roleFilter === "all" ? t("common.all") : roleFilter}
                options={[{ value: "all", label: t("common.all") }, ...roles.map((r) => ({ value: r, label: r }))]}
                onChange={setRoleFilter}
                openFilter={openFilter}
                setOpenFilter={setOpenFilter}
              />
              <PickerFilter
                id="geo"
                label={t("accountPicker.geo")}
                value={getGeoLabel(geoFilter)}
                options={geoOptions}
                onChange={setGeoFilter}
                openFilter={openFilter}
                setOpenFilter={setOpenFilter}
              />
              <PickerFilter
                id="aging"
                label={t("accounts.aging")}
                value={t(`accounts.aging${agingFilter.charAt(0).toUpperCase()}${agingFilter.slice(1)}` as any)}
                options={Object.entries(agingLabels).map(([k]) => ({ value: k, label: t(`accounts.aging${k.charAt(0).toUpperCase()}${k.slice(1)}` as any) }))}
                onChange={(v) => setAgingFilter(v as AgingFilter)}
                openFilter={openFilter}
                setOpenFilter={setOpenFilter}
              />
              <TriToggle label="Premium" value={filterPremium} onChange={setFilterPremium} allLabel={t("filters.all")} yesLabel={t("filters.yes")} noLabel={t("filters.no")} />
              <TriToggle label="2FA" value={filterTwoFa} onChange={setFilterTwoFa} allLabel={t("filters.all")} yesLabel={t("filters.yes")} noLabel={t("filters.no")} />
              <TriToggle label="Username" value={filterHasUsername} onChange={setFilterHasUsername} allLabel={t("filters.all")} yesLabel={t("filters.yes")} noLabel={t("filters.no")} />
              <div className="flex items-center gap-1 rounded-md border border-border bg-background px-2 py-1">
                <span className="text-xs text-muted-foreground mr-1">{t("filters.status")}:</span>
                {["clean", "spamblock", "frozen", "invalid", "unchecked"].map((s) => (
                  <button
                    key={s}
                    onClick={() => setFilterStatus((prev) => {
                      const next = new Set(prev);
                      if (next.has(s)) next.delete(s); else next.add(s);
                      return next;
                    })}
                    className={`rounded px-1.5 py-0.5 text-[11px] font-medium transition ${filterStatus.has(s) ? "bg-primary/20 text-primary" : "text-muted-foreground hover:text-foreground"}`}
                  >
                    {t(`filters.${s}` as any)}
                  </button>
                ))}
              </div>
              <div className="flex items-center gap-1 rounded-md border border-border bg-background px-2 py-1">
                <span className="text-xs text-muted-foreground mr-1">{t("filters.idDigits")}:</span>
                <input type="number" placeholder={t("filters.from")} value={filterIdMin} onChange={(e) => setFilterIdMin(e.target.value)} className="w-12 bg-transparent text-[11px] outline-none placeholder:text-muted-foreground/50" />
                <span className="text-muted-foreground text-[11px]">—</span>
                <input type="number" placeholder={t("filters.to")} value={filterIdMax} onChange={(e) => setFilterIdMax(e.target.value)} className="w-12 bg-transparent text-[11px] outline-none placeholder:text-muted-foreground/50" />
              </div>
              {activeFilterCount > 0 && (
                <button
                  onClick={resetAdvancedFilters}
                  className="rounded-md border border-border bg-background px-2 py-1 text-[11px] text-muted-foreground hover:text-foreground hover:border-primary/50 transition"
                >
                  {t("filters.reset")}
                </button>
              )}
            </div>
          )}
        </div>

        {/* table */}
        <div className="flex-1 overflow-y-auto scrollbar-thin min-h-[300px]">
          <table className="w-full text-sm">
            <thead className="bg-card border-b border-border sticky top-0 z-10">
              <tr>
                <th className="w-10 px-3 py-3">
                  <input type="checkbox" checked={allSelected} onChange={toggleAll} className="rounded border-border" />
                </th>
                <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accountPicker.phone")}</th>
                <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accountPicker.geo")}</th>
                <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accountPicker.status")}</th>
                <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accountPicker.role")}</th>
                <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accountPicker.name")}</th>
              </tr>
            </thead>
            <tbody>
              {filtered.slice(page * pageSize, (page + 1) * pageSize).map((acc) => (
                <tr
                  key={acc.id}
                  className={`border-b border-border transition cursor-pointer ${selected.has(acc.id) ? "bg-primary/5" : "hover:bg-card/60"}`}
                  onClick={() => toggleOne(acc.id)}
                >
                  <td className="px-3 py-3">
                    <input type="checkbox" checked={selected.has(acc.id)} readOnly className="rounded border-border pointer-events-none" />
                  </td>
                  <td className="px-3 py-3 font-mono text-xs">
                    {acc.phone ? (
                      <button
                        type="button"
                        onClick={(e) => { e.stopPropagation(); copyCell(acc.id, "phone", `+${acc.phone}`); }}
                        title={t("accounts.copyTooltip")}
                        className={`inline-flex items-center rounded px-1.5 py-0.5 transition cursor-pointer hover:bg-accent/40 hover:text-foreground ${copiedCellKey === `${acc.id}:phone` ? "text-primary" : ""}`}
                      >
                        {copiedCellKey === `${acc.id}:phone` ? t("accountPicker.copied") : `+${acc.phone}`}
                      </button>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="px-3 py-3">
                    {acc.geo ? (
                      <span className="inline-flex items-center gap-1.5">
                        {acc.geo === "ANON" ? (
                          <span className="text-base" title={t("accounts.anonNumber")}>🏴‍☠️</span>
                        ) : (
                          <img src={`https://flagcdn.com/16x12/${acc.geo.toLowerCase()}.png`} alt={acc.geo} className="w-4 h-3 object-cover rounded-sm" />
                        )}
                        <span className="text-xs">{getCountryName(acc.geo)}</span>
                      </span>
                    ) : <span className="text-muted-foreground">—</span>}
                  </td>
                  <td className="px-3 py-3"><StatusDot status={acc.status} /></td>
                  <td className="px-3 py-3">
                    {acc.role ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-muted/50 border border-border px-2 py-0.5 text-xs font-medium text-foreground/80">
                        <Tag className="h-3 w-3 text-muted-foreground" />
                        {acc.role}
                      </span>
                    ) : <span className="text-muted-foreground">—</span>}
                  </td>
                  <td className="px-3 py-3">
                    <span className="inline-flex items-center gap-1">
                      {acc.name || "—"}
                      {acc.username && (
                        <span className="text-muted-foreground text-xs">@{acc.username}</span>
                      )}
                      {acc.premium && (
                        <svg className="inline-block shrink-0" width="14" height="15" viewBox="0 0 14 15" fill="none">
                          <path fillRule="evenodd" clipRule="evenodd" d="M6.63869 12.1902L3.50621 14.1092C3.18049 14.3087 2.75468 14.2064 2.55515 13.8807C2.45769 13.7216 2.42864 13.5299 2.47457 13.3491L2.95948 11.4405C3.13452 10.7515 3.60599 10.1756 4.24682 9.86791L7.6642 8.22716C7.82352 8.15067 7.89067 7.95951 7.81418 7.80019C7.75223 7.67116 7.61214 7.59896 7.47111 7.62338L3.66713 8.28194C2.89387 8.41581 2.1009 8.20228 1.49941 7.69823L0.297703 6.69116C0.00493565 6.44581 -0.0335059 6.00958 0.211842 5.71682C0.33117 5.57442 0.502766 5.48602 0.687982 5.47153L4.35956 5.18419C4.61895 5.16389 4.845 4.99974 4.94458 4.75937L6.36101 1.3402C6.5072 0.987302 6.91179 0.819734 7.26469 0.965925C7.43413 1.03612 7.56876 1.17075 7.63896 1.3402L9.05539 4.75937C9.15496 4.99974 9.38101 5.16389 9.6404 5.18419L13.3322 5.47311C13.713 5.50291 13.9975 5.83578 13.9677 6.2166C13.9534 6.39979 13.8667 6.56975 13.7269 6.68896L10.9114 9.08928C10.7131 9.25826 10.6267 9.52425 10.6876 9.77748L11.5532 13.3733C11.6426 13.7447 11.414 14.1182 11.0427 14.2076C10.8642 14.2506 10.676 14.2208 10.5195 14.1249L7.36128 12.1902C7.13956 12.0544 6.8604 12.0544 6.63869 12.1902Z" fill="oklch(0.7 0.15 220)"/>
                        </svg>
                      )}
                    </span>
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr><td colSpan={6} className="text-center py-12 text-muted-foreground text-sm">{t("accountPicker.noAccounts")}</td></tr>
              )}
            </tbody>
          </table>
        </div>

        {/* footer */}
        <div className="px-6 py-4 border-t border-border flex items-center justify-between">
          <span className="text-sm text-muted-foreground">
            {selected.size > 0 ? t("accountPicker.selectedOf", { selected: selected.size, total: filtered.length }) : t("accountPicker.totalCount", { count: filtered.length })}
          </span>
          <div className="flex items-center gap-3">
            {filtered.length > pageSize && (
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setPage((p) => Math.max(0, p - 1))}
                  disabled={page === 0}
                  className="rounded-md border border-border bg-card px-2.5 py-1 text-xs disabled:opacity-30 hover:border-primary/50 transition"
                >
                  {t("common.back")}
                </button>
                <span className="text-xs text-muted-foreground">
                  {page * pageSize + 1}–{Math.min((page + 1) * pageSize, filtered.length)} / {filtered.length}
                </span>
                <button
                  onClick={() => setPage((p) => Math.min(Math.ceil(filtered.length / pageSize) - 1, p + 1))}
                  disabled={(page + 1) * pageSize >= filtered.length}
                  className="rounded-md border border-border bg-card px-2.5 py-1 text-xs disabled:opacity-30 hover:border-primary/50 transition"
                >
                  {t("common.forward")}
                </button>
              </div>
            )}
            <button
              onClick={onClose}
              className="rounded-md border border-border px-4 py-2 text-sm hover:bg-accent/50 transition"
            >
              {t("common.cancel")}
            </button>
            <button
              onClick={() => { onSelect(Array.from(selected)); onClose(); }}
              disabled={selected.size === 0}
              className="rounded-md border border-primary/50 bg-primary/10 px-4 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-30 disabled:cursor-not-allowed"
            >
              {t("accountPicker.select", { count: selected.size })}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: string }) {
  const colors: Record<string, string> = {
    "Без ограничений": "oklch(0.65 0.1 150)",
    "Заморожен": "oklch(0.65 0.1 220)",
    "Вечный спамблок": "oklch(0.55 0.1 25)",
    "Невалид": "oklch(0.55 0.1 25)",
    "Не проверен": "oklch(0.6 0.05 300)",
    "TData (не конвертирован)": "oklch(0.6 0.1 60)",
  };
  const isChecking = status.startsWith("Проверка");
  const isTempSpam = status.startsWith("Спамблок по ГЕО");
  const color = isChecking ? "oklch(0.65 0.1 280)"
    : isTempSpam ? "oklch(0.7 0.15 85)"
    : (colors[status] || "oklch(0.6 0.05 300)");

  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium ${isChecking ? "animate-pulse" : ""}`}
      style={{
        color,
        borderColor: `color-mix(in oklch, ${color} 30%, transparent)`,
        background: `color-mix(in oklch, ${color} 6%, transparent)`,
      }}
    >
      <span className="h-1.5 w-1.5 rounded-full" style={{ background: color }} />
      {status}
    </span>
  );
}

function PickerFilter({ id, label, value, options, onChange, openFilter, setOpenFilter }: {
  id: string;
  label: string;
  value: string;
  options: GeoOption[];
  onChange: (v: string) => void;
  openFilter: string | null;
  setOpenFilter: (v: string | null) => void;
}) {
  const isOpen = openFilter === id;
  const ref = useRef<HTMLDivElement>(null);
  const [dropSearch, setDropSearch] = useState("");
  const searchable = id === "geo";

  useEffect(() => {
    if (!isOpen) { setDropSearch(""); return; }
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpenFilter(null);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [isOpen]);

  const filtered = searchable && dropSearch
    ? options.filter((o) => o.separator || getGeoSearchText(o).includes(dropSearch.toLowerCase()))
    : options;

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpenFilter(isOpen ? null : id)}
        className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
      >
        {label}: {value}
        <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
      </button>
      {isOpen && (
        <div className={`absolute top-full left-0 mt-1 z-50 max-h-80 overflow-y-auto scrollbar-thin rounded-md border border-border bg-card shadow-lg py-1 ${searchable ? "min-w-[240px]" : "min-w-[140px]"}`}>
          {searchable && (
            <div className="px-2 pb-1.5 pt-1 sticky top-0 bg-card z-10">
              <div className="relative">
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground" />
                <input
                  value={dropSearch}
                  onChange={(e) => setDropSearch(e.target.value)}
                  placeholder="Поиск..."
                  className="w-full rounded border border-border bg-background pl-7 pr-2 py-1 text-xs outline-none focus:border-primary/50"
                  autoFocus
                />
              </div>
            </div>
          )}
          {filtered.map((o, i) => (
            o.separator ? (
              <div key={`sep-${i}`} className="border-t border-border my-1" />
            ) : (
              <button
                key={o.value}
                onClick={() => { onChange(o.value); setOpenFilter(null); }}
                className={`w-full text-left px-3 py-1.5 text-sm hover:bg-accent/50 ${o.label === value ? "text-primary font-medium" : ""}`}
              >
                {o.label}
              </button>
            )
          ))}
        </div>
      )}
    </div>
  );
}

function matchAging(aging: string, filter: AgingFilter): boolean {
  const days = agingToDays(aging);
  switch (filter) {
    case "day": return days <= 1;
    case "week": return days <= 7;
    case "month": return days <= 30;
    case "year": return days <= 365;
    case "more": return days > 365;
    default: return true;
  }
}

function agingToDays(aging: string): number {
  let days = 0;
  const yearMatch = aging.match(/(\d+)\s*г/);
  const monthMatch = aging.match(/(\d+)\s*мес/);
  const dayMatch = aging.match(/(\d+)\s*дн/);
  const hourMatch = aging.match(/(\d+)\s*ч/);
  if (yearMatch) days += parseInt(yearMatch[1]) * 365;
  if (monthMatch) days += parseInt(monthMatch[1]) * 30;
  if (dayMatch) days += parseInt(dayMatch[1]);
  if (hourMatch) days += parseInt(hourMatch[1]) / 24;
  return days;
}

function TriToggle({ label, value, onChange, allLabel, yesLabel, noLabel }: { label: string; value: "all" | "yes" | "no"; onChange: (v: "all" | "yes" | "no") => void; allLabel?: string; yesLabel?: string; noLabel?: string }) {
  return (
    <div className="flex items-center gap-1 rounded-md border border-border bg-background px-2 py-1">
      <span className="text-xs text-muted-foreground mr-1">{label}:</span>
      <button onClick={() => onChange("all")} className={`rounded px-1.5 py-0.5 text-[11px] font-medium transition ${value === "all" ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground"}`}>{allLabel || "All"}</button>
      <button onClick={() => onChange("yes")} className={`rounded px-1.5 py-0.5 text-[11px] font-medium transition ${value === "yes" ? "bg-primary/20 text-primary" : "text-muted-foreground hover:text-foreground"}`}>{yesLabel || "Yes"}</button>
      <button onClick={() => onChange("no")} className={`rounded px-1.5 py-0.5 text-[11px] font-medium transition ${value === "no" ? "bg-destructive/20 text-destructive" : "text-muted-foreground hover:text-foreground"}`}>{noLabel || "No"}</button>
    </div>
  );
}
