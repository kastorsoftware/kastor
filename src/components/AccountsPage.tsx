import { useState, useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import {
  Users, ShieldCheck, ShieldAlert, ChevronDown,
  Tag, Search, Plus, FolderOpen, X, Send, Filter,
} from "lucide-react";
import { ThreadInput } from "@/components/ThreadInput";
import { StatBlock } from "@/components/accounts/AccountStats";
import { StatusPill } from "@/components/accounts/StatusPill";
import { FilterDropdown } from "@/components/accounts/FilterDropdown";
import { ProxyDistributeDialog } from "@/components/accounts/ProxyDistributeDialog";
import { AuthKeyImportModal } from "@/components/accounts/AuthKeyImportModal";
import { PhoneLoginModal } from "@/components/accounts/PhoneLoginModal";
import {
  getGeoLabel,
  getCountryName,
  buildGeoOptions,
  CIS_COUNTRIES,
  ASIA_COUNTRIES,
  EUROPE_COUNTRIES,
  AMERICAS_COUNTRIES,
  AFRICA_COUNTRIES,
  MIDDLE_EAST_COUNTRIES,
} from "@/lib/countryData";
import { useT } from "@/i18n";

interface AccountData {
  id: string;
  phone: string;
  geo: string;
  status: string;
  aging: string;
  role: string;
  name: string;
  username: string;
  app_id?: number;
  proxy?: string;
  two_fa: string;
  premium: string;
  user_id: number;
}

interface AccountsStats {
  total: number;
  clean: number;
  restricted: number;
}

type AgingFilter = "all" | "day" | "week" | "month" | "year" | "more";
type GeoFilter = string;

const agingLabels: Record<AgingFilter, string> = {
  all: "all",
  day: "day",
  week: "week",
  month: "month",
  year: "year",
  more: "more",
};

export function AccountsPage() {
  const t = useT();
  const geoOptions = useMemo(() => buildGeoOptions(), [t]);
  const [accounts, setAccounts] = useState<AccountData[]>([]);
  const [stats, setStats] = useState<AccountsStats>({ total: 0, clean: 0, restricted: 0 });
  const [roles, setRoles] = useState<string[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [roleFilter, setRoleFilter] = useState("all");
  const [geoFilter, setGeoFilter] = useState<GeoFilter>("all");
  const [agingFilter, setAgingFilter] = useState<AgingFilter>("all");
  const [search, setSearch] = useState("");
  const [showFilters, setShowFilters] = useState(false);
  const [filterPremium, setFilterPremium] = useState<"all" | "yes" | "no">("all");
  const [filterTwoFa, setFilterTwoFa] = useState<"all" | "yes" | "no">("all");
  const [filterDetailedStatus, setFilterDetailedStatus] = useState<Set<string>>(new Set());
  const [filterHasUsername, setFilterHasUsername] = useState<"all" | "yes" | "no">("all");
  const [filterIdMin, setFilterIdMin] = useState("");
  const [filterIdMax, setFilterIdMax] = useState("");
  const [roleDropdownOpen, setRoleDropdownOpen] = useState(false);
  const [validateDropdownOpen, setValidateDropdownOpen] = useState(false);
  const [validateAllDropdownOpen, setValidateAllDropdownOpen] = useState(false);
  const [reauthDropdownOpen, setReauthDropdownOpen] = useState(false);
  const [reauthThreads, setReauthThreads] = useState(5);
  const [reauthTerminateOthers, setReauthTerminateOthers] = useState(false);
  const [reauthRunning, setReauthRunning] = useState(false);
  const [reauthResults, setReauthResults] = useState<{ success: number; unknown_2fa: number; failed: number; errors: string[] } | null>(null);
  const [newRoleName, setNewRoleName] = useState("");
  const [openFilter, setOpenFilter] = useState<string | null>(null);
  const [importDialogOpen, setImportDialogOpen] = useState(false);
  const [authKeyDialogOpen, setAuthKeyDialogOpen] = useState(false);
  const [phoneLoginOpen, setPhoneLoginOpen] = useState(false);
  const [missingJsonWarning, setMissingJsonWarning] = useState(false);
  const [duplicateWarning, setDuplicateWarning] = useState(0);
  const [multiAccountWarning, setMultiAccountWarning] = useState(0);
  const [passcodeWarning, setPasscodeWarning] = useState(0);
  const [validating, setValidating] = useState(false);
  const [validatingIds, setValidatingIds] = useState<Set<string>>(new Set());
  const [checkRestrictions, setCheckRestrictions] = useState(true);
  const [check2fa, setCheck2fa] = useState(false);
  const [validateThreads, setValidateThreads] = useState(5);

  // load saved thread count from settings
  useEffect(() => {
    if (IS_DEV) return;
    invoke<{ account_threads?: number; reauth_threads?: number; validate_check_2fa?: boolean }>("get_settings").then((s) => {
      if (s.account_threads) setValidateThreads(s.account_threads);
      if (s.reauth_threads) setReauthThreads(s.reauth_threads);
      if (s.validate_check_2fa !== undefined) setCheck2fa(s.validate_check_2fa);
    }).catch(() => {});
  }, []);
  const [threadsWarning, setThreadsWarning] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const [proxyDistributeOpen, setProxyDistributeOpen] = useState(false);
  const [pendingValidateAfterDistribute, setPendingValidateAfterDistribute] = useState(false);
  const [pendingValidateIds, setPendingValidateIds] = useState<string[]>([]);
  const [validateResult, setValidateResult] = useState<{ valid: number; invalid: number; restricted: number; unreachable: number; errors: string[] } | null>(null);
  const [noProxyWarning, setNoProxyWarning] = useState(false);
  const [page, setPage] = useState(0);
  const pageSize = 30;
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const checkedIdsRef = useRef<Set<string>>(new Set());
  const roleDropdownRef = useRef<HTMLDivElement>(null);
  const validateDropdownRef = useRef<HTMLDivElement>(null);
  const validateAllDropdownRef = useRef<HTMLDivElement>(null);
  const reauthDropdownRef = useRef<HTMLDivElement>(null);

  // close dropdowns on click outside
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (roleDropdownOpen && roleDropdownRef.current && !roleDropdownRef.current.contains(target)) {
        setRoleDropdownOpen(false);
      }
      if (validateDropdownOpen && validateDropdownRef.current && !validateDropdownRef.current.contains(target)) {
        setValidateDropdownOpen(false);
      }
      if (validateAllDropdownOpen && validateAllDropdownRef.current && !validateAllDropdownRef.current.contains(target)) {
        setValidateAllDropdownOpen(false);
      }
      if (reauthDropdownOpen && reauthDropdownRef.current && !reauthDropdownRef.current.contains(target)) {
        setReauthDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [roleDropdownOpen, validateDropdownOpen, validateAllDropdownOpen, reauthDropdownOpen]);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  const refreshAccounts = async () => {
    if (IS_DEV) return;
    const data = await invoke<{ accounts: AccountData[]; stats: AccountsStats }>("get_accounts_with_stats");
    setAccounts(data.accounts);
    setStats(data.stats);
  };

  useEffect(() => {
    if (IS_DEV) {
      setAccounts([]);
      setStats({ total: 0, clean: 0, restricted: 0 });
      setRoles([]);
      return;
    }
    refreshAccounts();
    invoke<string[]>("get_roles").then(setRoles);

    // listen for backend event signaling account data changed
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<void>("accounts-changed", () => {
        refreshAccounts();
      }).then((fn) => { unlisten = fn; });
    });

    // fallback refresh every 3 seconds (covers manual json edits)
    const interval = setInterval(refreshAccounts, 3000);
    return () => {
      clearInterval(interval);
      if (unlisten) unlisten();
    };
  }, []);

  // poll validating ids while validation is running
  useEffect(() => {
    if (!validating || IS_DEV) return;

    checkedIdsRef.current = new Set(validatingIds);

    pollRef.current = setInterval(async () => {
      const ids = await invoke<string[]>("get_validating_ids");
      setValidatingIds(new Set(ids));
      await refreshAccounts();

      if (ids.length === 0) {
        setValidating(false);
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = null;

        // compute results from accounts that were being validated
        const updated = await invoke<AccountData[]>("get_real_accounts");
        const checked = updated.filter((a) => checkedIdsRef.current.has(a.id));
        const valid = checked.filter((a) => a.status === "Без ограничений" || a.status === "No restrictions").length;
        const invalid = checked.filter((a) => a.status === "Невалид" || a.status === "Invalid").length;
        const restricted = checked.filter((a) => a.status === "Вечный спамблок" || a.status === "Заморожен" || a.status.startsWith("Спамблок по ГЕО") || a.status === "Permanent spamblock" || a.status === "Frozen" || a.status.startsWith("Geo spamblock")).length;
        const unreachable = checked.filter((a) => a.status === "Не проверен" || a.status.startsWith("Проверка") || a.status === "Unchecked" || a.status.startsWith("Checking")).length;
        const errAccounts = checked.filter((a) => a.status === "Не проверен" || a.status === "Unchecked");
        const errors = errAccounts.length > 0 ? [t("accounts.proxyConnectionError")] : [];
        setValidateResult({ valid, invalid, restricted, unreachable, errors });
      }
    }, 1000);

    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, [validating]);

  const filtered = useMemo(() => {
    return accounts.filter((a) => {
      if (roleFilter !== "all" && a.role !== roleFilter) return false;
      if (geoFilter !== "all") {
        if (geoFilter === "cis" && !CIS_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "non-cis" && CIS_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "asia" && !ASIA_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "europe" && !EUROPE_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "americas" && !AMERICAS_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "africa" && !AFRICA_COUNTRIES.includes(a.geo)) return false;
        if (geoFilter === "middle-east" && !MIDDLE_EAST_COUNTRIES.includes(a.geo)) return false;
        // individual country filter
        if (geoFilter.length === 2 && geoFilter === geoFilter.toUpperCase()
            && geoFilter !== "cis" && geoFilter !== "non-cis" && geoFilter !== "asia"
            && geoFilter !== "europe" && geoFilter !== "americas") {
          if (a.geo !== geoFilter) return false;
        }
      }
      if (agingFilter !== "all") {
        if (!matchAging(a.aging, agingFilter)) return false;
      }
      if (search) {
        const s = search.toLowerCase().replace(/^\+/, "");
        const inPhone = a.phone.toLowerCase().includes(s);
        const inName = a.name.toLowerCase().includes(s);
        const inUsername = a.username.toLowerCase().includes(s);
        if (!inPhone && !inName && !inUsername) return false;
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
      // Detailed status filter (multi-select)
      if (filterDetailedStatus.size > 0) {
        const s = a.status;
        const matchesAny = Array.from(filterDetailedStatus).some((f) => {
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
  }, [accounts, roleFilter, geoFilter, agingFilter, search, filterPremium, filterTwoFa, filterHasUsername, filterDetailedStatus, filterIdMin, filterIdMax]);

  const [sortCol, setSortCol] = useState<string | null>(null);
  const [sortAsc, setSortAsc] = useState(true);

  const handleSort = (col: string) => {
    if (sortCol === col) {
      if (!sortAsc) { setSortCol(null); setSortAsc(true); }
      else setSortAsc(false);
    } else {
      setSortCol(col);
      setSortAsc(true);
    }
  };

  // precompute original index for O(1) lookup
  const accountIndexMap = useMemo(() => {
    const map = new Map<string, number>();
    accounts.forEach((a, i) => map.set(a.id, i + 1));
    return map;
  }, [accounts]);

  const sorted = useMemo(() => {
    if (!sortCol) return filtered;
    const statusOrder: Record<string, number> = {
      "Без ограничений": 0, "No restrictions": 0, "Спамблок по ГЕО": 1, "Geo spamblock": 1, "Заморожен": 3, "Frozen": 3, "Вечный спамблок": 2, "Permanent spamblock": 2, "Невалид": 4, "Invalid": 4, "Не проверен": 5, "Unchecked": 5,
    };
    const arr = [...filtered];
    arr.sort((a, b) => {
      let cmp = 0;
      switch (sortCol) {
        case "geo": cmp = getCountryName(a.geo).localeCompare(getCountryName(b.geo)); break;
        case "aging": cmp = agingToDays(b.aging) - agingToDays(a.aging); break;
        case "role": cmp = (b.role ? 1 : 0) - (a.role ? 1 : 0) || a.role.localeCompare(b.role, "ru"); break;
        case "status": cmp = (statusOrder[a.status] ?? 99) - (statusOrder[b.status] ?? 99); break;
        case "name": cmp = a.name.localeCompare(b.name, "ru"); break;
        case "username": cmp = (b.username ? 1 : 0) - (a.username ? 1 : 0) || a.username.localeCompare(b.username); break;
        case "two_fa": cmp = (b.two_fa ? 1 : 0) - (a.two_fa ? 1 : 0) || a.two_fa.localeCompare(b.two_fa); break;
        case "premium": cmp = (b.premium ? 1 : 0) - (a.premium ? 1 : 0) || b.premium.localeCompare(a.premium); break;
        case "#": cmp = 0; break;
      }
      return sortAsc ? cmp : -cmp;
    });
    return arr;
  }, [filtered, sortCol, sortAsc]);

  const allSelected = sorted.length > 0 && sorted.every((a) => selected.has(a.id));
  const hasSelection = selected.size > 0;

  const toggleAll = () => {
    if (allSelected) {
      setSelected(new Set());
    } else {
      setSelected(new Set(sorted.map((a) => a.id)));
    }
  };

  const toggleOne = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const assignRole = async (role: string) => {
    if (IS_DEV) return;
    await invoke("assign_role", { ids: Array.from(selected), role });
    await refreshAccounts();
    setRoleDropdownOpen(false);
  };

  const removeRole = async () => {
    if (IS_DEV) return;
    await invoke("assign_role", { ids: Array.from(selected), role: "" });
    await refreshAccounts();
    setRoleDropdownOpen(false);
  };

  const addNewRole = async () => {
    if (!newRoleName.trim() || IS_DEV) return;
    await invoke("add_role", { name: newRoleName.trim() });
    const updated = await invoke<string[]>("get_roles");
    setRoles(updated);
    setNewRoleName("");
  };

  const deleteRole = async (role: string) => {
    if (IS_DEV) return;
    await invoke("delete_role", { name: role });
    const updated = await invoke<string[]>("get_roles");
    setRoles(updated);
    if (roleFilter === role) setRoleFilter("all");
  };

  const handleValidate = async () => {
    if (IS_DEV) return;

    // check if allow_no_proxy is disabled and accounts have no proxy
    const settings = await invoke<{ allow_no_proxy: boolean }>("get_settings").catch(() => ({ allow_no_proxy: false }));
    if (!settings.allow_no_proxy) {
      const ids = Array.from(selected);
      const allHaveProxy = await invoke<boolean>("check_accounts_have_proxy", { ids });
      if (!allHaveProxy) {
        setNoProxyWarning(true);
        return;
      }
    }

    const ids = Array.from(selected);
    setValidatingIds(new Set(ids));
    setValidating(true);
    setSelected(new Set());
    await invoke("enqueue_validate", { ids, checkRestrictions, check2fa, checkAging: true, threads: validateThreads });
    await refreshAccounts();
  };

  const handleValidateAll = async () => {
    if (IS_DEV) return;

    const ids = accounts.map((a) => a.id);
    if (ids.length === 0) return;

    const settings = await invoke<{ allow_no_proxy: boolean }>("get_settings").catch(() => ({ allow_no_proxy: false }));
    if (!settings.allow_no_proxy) {
      const allHaveProxy = await invoke<boolean>("check_accounts_have_proxy", { ids });
      if (!allHaveProxy) {
        setNoProxyWarning(true);
        return;
      }
    }

    setValidatingIds(new Set(ids));
    setValidating(true);
    setSelected(new Set());
    await invoke("enqueue_validate", { ids, checkRestrictions, check2fa, checkAging: true, threads: validateThreads });
    await refreshAccounts();
  };

  const handleReauth = async () => {
    if (IS_DEV) return;
    const ids = Array.from(selected);
    setReauthRunning(true);
    setSelected(new Set());
    try {
      const res = await invoke<{ success: number; unknown_2fa: number; failed: number; errors: string[] }>("reauth_accounts", { ids, threads: reauthThreads, terminateOthers: reauthTerminateOthers });
      setReauthResults(res);
    } catch (e: any) {
      setReauthResults({ success: 0, unknown_2fa: 0, failed: ids.length, errors: [String(e)] });
    } finally {
      setReauthRunning(false);
      await refreshAccounts();
    }
  };

  const handleDeleteAccounts = async () => {
    if (IS_DEV) return;
    const ids = Array.from(selected);
    await invoke("delete_accounts", { ids });
    setDeleteConfirm(false);
    setSelected(new Set());
    await refreshAccounts();
  };

  const [deleteOneId, setDeleteOneId] = useState<string | null>(null);
  const [editing2faId, setEditing2faId] = useState<string | null>(null);
  const [editing2faValue, setEditing2faValue] = useState("");
  // copy feedback uses a single cell key "<accId>:<field>" so multiple cells
  // can be made copyable without growing per-field state.
  const [copiedCellKey, setCopiedCellKey] = useState<string | null>(null);

  const copyCell = async (accId: string, field: string, text: string) => {
    if (!text) return;
    try {
      await writeText(text);
    } catch {}
    const key = `${accId}:${field}`;
    setCopiedCellKey(key);
    setTimeout(() => {
      setCopiedCellKey((prev) => (prev === key ? null : prev));
    }, 1200);
  };

  const handleDeleteOne = async (id: string) => {
    setDeleteOneId(id);
  };

  const confirmDeleteOne = async () => {
    if (IS_DEV || !deleteOneId) return;
    await invoke("delete_accounts", { ids: [deleteOneId] });
    setDeleteOneId(null);
    await refreshAccounts();
  };

  const handleImport = async (format: string) => {
    setImportDialogOpen(false);
    if (IS_DEV) return;

    const { open } = await import("@tauri-apps/plugin-dialog");

    let selected_paths;
    if (format === "tdata") {
      selected_paths = await open({ multiple: true, directory: true });
    } else if (format === "tdata_zip") {
      selected_paths = await open({
        multiple: true,
        filters: [{ name: "TData archives", extensions: ["zip"] }],
      });
    } else {
      selected_paths = await open({
        multiple: true,
        filters: [{ name: "Session files", extensions: ["session"] }],
      });
    }

    if (!selected_paths) return;
    const paths = Array.isArray(selected_paths) ? selected_paths : [selected_paths];

    const actualFormat = format === "tdata_zip" ? "tdata" : format;
    const results = await invoke<{ id: string; success: boolean; missing_json: boolean; error: string | null; multi_account_split: boolean }[]>(
      "import_accounts", { paths, format: actualFormat }
    );

    // check for duplicates
    const dupeCount = results.filter((r) => !r.success && r.error === "duplicate").length;
    if (dupeCount > 0) {
      setDuplicateWarning(dupeCount);
    }

    // check for local passcode protected tdata
    const passcodeCount = results.filter((r) => !r.success && r.error === "local_passcode").length;
    if (passcodeCount > 0) {
      setPasscodeWarning(passcodeCount);
    }

    // check for multi-account tdata splits
    const multiCount = results.filter((r) => r.success && r.multi_account_split).length;
    if (multiCount > 0) {
      setMultiAccountWarning(multiCount);
    }

    // check if any imported accounts are missing json
    const hasMissing = results.some((r) => r.success && r.missing_json);
    if (hasMissing) {
      setMissingJsonWarning(true);
    }

    await refreshAccounts();
  };

  const handleOpenFolder = async () => {
    if (IS_DEV) return;
    await invoke("open_accounts_folder");
  };

  const activeFilterCount = [
    roleFilter !== "all",
    geoFilter !== "all",
    agingFilter !== "all",
    filterPremium !== "all",
    filterTwoFa !== "all",
    filterDetailedStatus.size > 0,
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
    setFilterDetailedStatus(new Set());
    setFilterHasUsername("all");
    setFilterIdMin("");
    setFilterIdMax("");
  };

  // Reset page when advanced filters change
  useEffect(() => {
    setPage(0);
  }, [filterPremium, filterTwoFa, filterHasUsername, filterDetailedStatus, filterIdMin, filterIdMax]);

  return (
    <div className="space-y-6">
      {/* stat cards */}
      <div className="grid grid-cols-3 gap-4">
        <StatBlock icon={Users} label={t("accounts.allAccounts")} value={stats.total} active />
        <StatBlock icon={ShieldCheck} label={t("accounts.noRestrictions")} value={stats.clean} />
        <StatBlock icon={ShieldAlert} label={t("accounts.withRestrictions")} value={stats.restricted} />
      </div>

      {/* filters or bulk actions */}
      {hasSelection ? (
        <div className="flex items-center gap-3 rounded-md border border-primary/30 bg-card px-4 py-3">
          <span className="text-sm text-muted-foreground">{t("accounts.selected")}: {selected.size}</span>

          {/* left: delete + role */}
          <button
            onClick={() => setDeleteConfirm(true)}
            className="rounded-md border border-border bg-background px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-destructive hover:border-destructive/30 transition"
          >
            {t("accounts.deleteBtn", { count: selected.size })}
          </button>
          <div className="relative" ref={roleDropdownRef}>
            <button
              onClick={() => setRoleDropdownOpen(!roleDropdownOpen)}
              className="flex items-center gap-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
            >
              {t("accounts.assignRole")}
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
            {roleDropdownOpen && (
              <div className="absolute top-full left-0 mt-1 z-50 w-56 max-h-64 overflow-y-auto scrollbar-thin rounded-md border border-border bg-card shadow-lg py-1">
                <button
                  onClick={removeRole}
                  className="w-full text-left px-3 py-2 text-sm hover:bg-accent/50 text-destructive"
                >
                  {t("accounts.removeRole")}
                </button>
                <div className="border-t border-border my-1" />
                {roles.map((r) => (
                  <div key={r} className="flex items-center justify-between px-3 py-2 hover:bg-accent/50 group">
                    <button
                      onClick={() => assignRole(r)}
                      className="flex-1 text-left text-sm"
                    >
                      {r}
                    </button>
                    <button
                      onClick={(e) => { e.stopPropagation(); deleteRole(r); }}
                      className="text-xs text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-destructive"
                    >
                      {t("accounts.deleteRole")}
                    </button>
                  </div>
                ))}
                <div className="border-t border-border my-1" />
                <div className="flex items-center gap-2 px-3 py-2">
                  <input
                    value={newRoleName}
                    onChange={(e) => setNewRoleName(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && addNewRole()}
                    placeholder={t("accounts.newRole")}
                    className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/60"
                  />
                  <button onClick={addNewRole} className="text-xs text-primary font-medium">
                    +
                  </button>
                </div>
              </div>
            )}
          </div>

          {/* right: validate dropdown */}
          <div className="ml-auto relative" ref={validateDropdownRef}>
            <button
              onClick={() => { setValidateDropdownOpen(!validateDropdownOpen); setReauthDropdownOpen(false); setValidateAllDropdownOpen(false); }}
              className="flex items-center gap-1.5 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
            >
              {t("accounts.validation")}
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
            {validateDropdownOpen && (
              <div className="absolute top-full right-0 mt-1 z-50 w-64 rounded-md border border-border bg-card shadow-lg py-2 px-3 space-y-2.5"
                   onClick={(e) => e.stopPropagation()}>
                <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
                  <input
                    type="checkbox"
                    checked={checkRestrictions}
                    onChange={(e) => setCheckRestrictions(e.target.checked)}
                    className="rounded border-border accent-primary"
                  />
                  {t("accounts.restrictions")}
                </label>
                <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
                  <input
                    type="checkbox"
                    checked={check2fa}
                    onChange={(e) => { setCheck2fa(e.target.checked); if (!IS_DEV) invoke("patch_settings", { patch: { validate_check_2fa: e.target.checked } }).catch(() => {}); }}
                    className="rounded border-border accent-primary"
                  />
                  {t("accounts.twoFa")}
                </label>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
                  <ThreadInput
                    value={validateThreads}
                    onChange={(v) => {
                      if (v > 100 && validateThreads <= 100) {
                        setThreadsWarning(true);
                      }
                      setValidateThreads(v);
                      if (!IS_DEV) invoke("patch_settings", { patch: { account_threads: v } }).catch(() => {});
                    }}
                    min={1}
                    max={1000}
                  />
                </div>
                <button
                  onClick={() => { setValidateDropdownOpen(false); handleValidate(); }}
                  disabled={validating}
                  className="w-full rounded-md bg-primary/10 border border-primary/30 px-3 py-1.5 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
                >
                  {validating ? t("accounts.validating") : t("accounts.validateBtn")}
                </button>
              </div>
            )}
          </div>

          {/* reauth dropdown */}
          <div className="relative" ref={reauthDropdownRef}>
            <button
              onClick={() => { setReauthDropdownOpen(!reauthDropdownOpen); setValidateDropdownOpen(false); setValidateAllDropdownOpen(false); }}
              className="flex items-center gap-1.5 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
            >
              {t("accounts.reauth")}
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
            {reauthDropdownOpen && (
              <div className="absolute top-full right-0 mt-1 z-50 w-64 rounded-md border border-border bg-card shadow-lg py-2 px-3 space-y-2.5"
                   onClick={(e) => e.stopPropagation()}>
                <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
                  <input
                    type="checkbox"
                    checked={reauthTerminateOthers}
                    onChange={(e) => setReauthTerminateOthers(e.target.checked)}
                    className="rounded border-border accent-primary"
                  />
                  {t("accounts.terminateOthers")}
                </label>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
                  <ThreadInput
                    value={reauthThreads}
                    onChange={(v) => {
                      setReauthThreads(v);
                      if (!IS_DEV) invoke("patch_settings", { patch: { reauth_threads: v } }).catch(() => {});
                    }}
                    min={1}
                    max={1000}
                  />
                </div>
                <button
                  onClick={() => { setReauthDropdownOpen(false); handleReauth(); }}
                  disabled={reauthRunning}
                  className="w-full rounded-md bg-primary/10 border border-primary/30 px-3 py-1.5 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
                >
                  {reauthRunning ? t("accounts.reauthRunning") : t("accounts.reauthBtn")}
                </button>
              </div>
            )}
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-3 flex-wrap">
          {!hasSelection && (
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
          )}
          <div className="ml-auto flex items-center gap-2">
            {/* validate all dropdown */}
            <div className="relative" ref={validateAllDropdownRef}>
              <button
                onClick={() => setValidateAllDropdownOpen(!validateAllDropdownOpen)}
                className="flex items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
              >
                {t("accounts.validateAll")}
                <ChevronDown className="h-3.5 w-3.5" />
              </button>
              {validateAllDropdownOpen && (
                <div className="absolute top-full right-0 mt-1 z-50 w-64 rounded-md border border-border bg-card shadow-lg py-2 px-3 space-y-2.5"
                     onClick={(e) => e.stopPropagation()}>
                  <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
                    <input
                      type="checkbox"
                      checked={checkRestrictions}
                      onChange={(e) => setCheckRestrictions(e.target.checked)}
                      className="rounded border-border accent-primary"
                    />
                    {t("accounts.restrictions")}
                  </label>
                  <label className="flex items-center gap-2 text-sm text-muted-foreground font-medium cursor-pointer">
                    <input
                      type="checkbox"
                      checked={check2fa}
                      onChange={(e) => { setCheck2fa(e.target.checked); if (!IS_DEV) invoke("patch_settings", { patch: { validate_check_2fa: e.target.checked } }).catch(() => {}); }}
                      className="rounded border-border accent-primary"
                    />
                    {t("accounts.twoFa")}
                  </label>
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">{t("common.threads")}:</span>
                    <ThreadInput
                      value={validateThreads}
                      onChange={(v) => {
                        if (v > 100 && validateThreads <= 100) {
                          setThreadsWarning(true);
                        }
                        setValidateThreads(v);
                        if (!IS_DEV) invoke("patch_settings", { patch: { account_threads: v } }).catch(() => {});
                      }}
                      min={1}
                      max={1000}
                    />
                  </div>
                  <button
                    onClick={() => { setValidateAllDropdownOpen(false); handleValidateAll(); }}
                    disabled={validating}
                    className="w-full rounded-md bg-primary/10 border border-primary/30 px-3 py-1.5 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
                  >
                    {validating ? t("accounts.validating") : t("accounts.validateAll")}
                  </button>
                </div>
              )}
            </div>
            <button
              onClick={() => setProxyDistributeOpen(true)}
              className="flex items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
              title={t("accounts.distribute")}
            >
              {t("common.proxies")}
            </button>
            <button
              onClick={() => setImportDialogOpen(true)}
              className="flex h-8 w-8 items-center justify-center rounded-md border border-border bg-card text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
              title={t("accounts.importBtn")}
            >
              <Plus className="h-4 w-4" />
            </button>
            <button
              onClick={handleOpenFolder}
              className="flex h-8 w-8 items-center justify-center rounded-md border border-border bg-card text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
              title={t("accounts.openFolder")}
            >
              <FolderOpen className="h-4 w-4" />
            </button>
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("common.search")}
                className="rounded-md border border-border bg-card pl-9 pr-3 py-1.5 text-sm outline-none focus:border-primary/50 w-48"
              />
            </div>
          </div>
        </div>
      )}

      {/* expanded filters panel */}
      {!hasSelection && showFilters && (
        <div className="flex flex-wrap items-center gap-2">
          <FilterDropdown
            id="role"
            label={t("accounts.role")}
            value={roleFilter === "all" ? t("common.all") : roleFilter}
            options={[{ value: "all", label: t("common.all") }, ...roles.map((r) => ({ value: r, label: r }))]}
            onChange={setRoleFilter}
            openFilter={openFilter}
            setOpenFilter={setOpenFilter}
          />
          <FilterDropdown
            id="geo"
            label={t("accounts.geo")}
            value={getGeoLabel(geoFilter)}
            options={geoOptions}
            onChange={(v) => setGeoFilter(v)}
            openFilter={openFilter}
            setOpenFilter={setOpenFilter}
          />
          <FilterDropdown
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
                onClick={() => setFilterDetailedStatus((prev) => {
                  const next = new Set(prev);
                  if (next.has(s)) next.delete(s); else next.add(s);
                  return next;
                })}
                className={`rounded px-1.5 py-0.5 text-[11px] font-medium transition ${filterDetailedStatus.has(s) ? "bg-primary/20 text-primary" : "text-muted-foreground hover:text-foreground"}`}
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
            <button onClick={resetAdvancedFilters} className="rounded-md border border-border bg-background px-2 py-1 text-[11px] text-muted-foreground hover:text-foreground hover:border-primary/50 transition">
              {t("filters.reset")}
            </button>
          )}
        </div>
      )}

      {/* table */}
      <div className="rounded-md border border-border">
        <div className="max-h-[55vh] overflow-y-auto scrollbar-thin">
        <table className="w-full text-sm">
          <thead className="bg-card border-b border-border sticky top-0 z-10">
            <tr>
              <th className="w-10 px-3 py-3">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={toggleAll}
                  className="rounded border-border"
                />
              </th>
              <th className="w-10 px-2 py-3 text-left text-muted-foreground font-medium">#</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium">{t("accounts.name")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("geo")}>{t("accounts.geo")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("status")}>{t("accounts.status")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("aging")}>{t("accounts.aging")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("role")}>{t("accounts.role")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("name")}>{t("accounts.name")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("username")}>{t("accounts.username")}</th>
              <th className="px-3 py-3 text-left text-muted-foreground font-medium cursor-pointer hover:text-foreground" onClick={() => handleSort("two_fa")}>{t("accounts.twoFa")}</th>
              <th className="w-8"></th>
            </tr>
          </thead>
          <tbody>
            {sorted.slice(page * pageSize, (page + 1) * pageSize).map((acc) => {
              const isProcessing = validatingIds.has(acc.id);
              return (
                <tr
                  key={acc.id}
                  className={`group border-b border-border transition ${
                    selected.has(acc.id) ? "bg-primary/5" : "hover:bg-card/60"
                  }`}
                >
                  <td className="px-3 py-3">
                    <input
                      type="checkbox"
                      checked={selected.has(acc.id)}
                      onChange={() => toggleOne(acc.id)}
                      className="rounded border-border"
                      disabled={isProcessing}
                    />
                  </td>
                  <td className="px-2 py-3 text-muted-foreground">
                    {accountIndexMap.get(acc.id) || 0}
                  </td>
                  <td className="px-3 py-3 font-mono text-xs">
                    {acc.phone ? (
                      <button
                        type="button"
                        onClick={() => copyCell(acc.id, "phone", `+${acc.phone}`)}
                        title={t("accounts.copyTooltip")}
                        className={`inline-flex items-center rounded px-1.5 py-0.5 transition cursor-pointer hover:bg-accent/40 hover:text-foreground ${copiedCellKey === `${acc.id}:phone` ? "text-primary" : ""}`}
                      >
                        {copiedCellKey === `${acc.id}:phone` ? t("accounts.copied") : `+${acc.phone}`}
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
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                  <td className="px-3 py-3">
                    {isProcessing ? (
                      <StatusPill status={acc.status || "..."} />
                    ) : (
                      <StatusPill status={acc.status} />
                    )}
                  </td>
                  <td className="px-3 py-3 text-muted-foreground">
                    {acc.aging || "—"}
                  </td>
                  <td className="px-3 py-3">
                    {acc.role ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-muted/50 border border-border px-2 py-0.5 text-xs font-medium text-foreground/80">
                        <Tag className="h-3 w-3 text-muted-foreground" />
                        {acc.role}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                  <td className="px-3 py-3">
                    <span className="inline-flex items-center gap-1">
                      {acc.name || "—"}
                      {acc.premium && (
                        <svg className="inline-block shrink-0" width="14" height="15" viewBox="0 0 14 15" fill="none">
                          <path fillRule="evenodd" clipRule="evenodd" d="M6.63869 12.1902L3.50621 14.1092C3.18049 14.3087 2.75468 14.2064 2.55515 13.8807C2.45769 13.7216 2.42864 13.5299 2.47457 13.3491L2.95948 11.4405C3.13452 10.7515 3.60599 10.1756 4.24682 9.86791L7.6642 8.22716C7.82352 8.15067 7.89067 7.95951 7.81418 7.80019C7.75223 7.67116 7.61214 7.59896 7.47111 7.62338L3.66713 8.28194C2.89387 8.41581 2.1009 8.20228 1.49941 7.69823L0.297703 6.69116C0.00493565 6.44581 -0.0335059 6.00958 0.211842 5.71682C0.33117 5.57442 0.502766 5.48602 0.687982 5.47153L4.35956 5.18419C4.61895 5.16389 4.845 4.99974 4.94458 4.75937L6.36101 1.3402C6.5072 0.987302 6.91179 0.819734 7.26469 0.965925C7.43413 1.03612 7.56876 1.17075 7.63896 1.3402L9.05539 4.75937C9.15496 4.99974 9.38101 5.16389 9.6404 5.18419L13.3322 5.47311C13.713 5.50291 13.9975 5.83578 13.9677 6.2166C13.9534 6.39979 13.8667 6.56975 13.7269 6.68896L10.9114 9.08928C10.7131 9.25826 10.6267 9.52425 10.6876 9.77748L11.5532 13.3733C11.6426 13.7447 11.414 14.1182 11.0427 14.2076C10.8642 14.2506 10.676 14.2208 10.5195 14.1249L7.36128 12.1902C7.13956 12.0544 6.8604 12.0544 6.63869 12.1902Z" fill="oklch(0.7 0.15 220)"/>
                        </svg>
                      )}
                    </span>
                  </td>
                  <td className="px-3 py-3 text-xs text-muted-foreground">
                    {acc.username ? (
                      <button
                        type="button"
                        onClick={() => copyCell(acc.id, "username", `@${acc.username}`)}
                        title={t("accounts.copyTooltip")}
                        className={`inline-flex items-center rounded px-1.5 py-0.5 transition cursor-pointer hover:bg-accent/40 hover:text-foreground ${copiedCellKey === `${acc.id}:username` ? "text-primary" : ""}`}
                      >
                        {copiedCellKey === `${acc.id}:username` ? t("accounts.copied") : `@${acc.username}`}
                      </button>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="px-3 py-3 text-xs">
                    {editing2faId === acc.id ? (
                      <input
                        autoFocus
                        value={editing2faValue}
                        onChange={(e) => setEditing2faValue(e.target.value)}
                        onBlur={async () => {
                          if (!IS_DEV) {
                            await invoke("set_account_two_fa", { id: acc.id, twoFa: editing2faValue });
                            await refreshAccounts();
                          }
                          setEditing2faId(null);
                        }}
                        onKeyDown={async (e) => {
                          if (e.key === "Enter") {
                            if (!IS_DEV) {
                              await invoke("set_account_two_fa", { id: acc.id, twoFa: editing2faValue });
                              await refreshAccounts();
                            }
                            setEditing2faId(null);
                          } else if (e.key === "Escape") {
                            setEditing2faId(null);
                          }
                        }}
                        className="w-full bg-transparent border-b border-primary/50 outline-none text-xs px-0 py-0"
                      />
                    ) : acc.two_fa ? (
                      <span
                        className="text-muted-foreground cursor-pointer truncate max-w-[80px] inline-block"
                        title={acc.two_fa}
                        onDoubleClick={() => { setEditing2faId(acc.id); setEditing2faValue(acc.two_fa.startsWith("Неизвестен") || acc.two_fa === "Установлен, неизвестен" || acc.two_fa.startsWith("Unknown") || acc.two_fa === "Set, unknown" ? "" : acc.two_fa); }}
                      >
                        {acc.two_fa.startsWith("Неизвестен") || acc.two_fa === "Установлен, неизвестен" || acc.two_fa.startsWith("Unknown") || acc.two_fa === "Set, unknown"
                          ? t("accounts.twoFaPresent")
                          : acc.two_fa.length > 10 ? t("accounts.twoFaHidden") : acc.two_fa}
                      </span>
                    ) : (
                      <span
                        className="text-muted-foreground/50 cursor-pointer"
                        onDoubleClick={() => { setEditing2faId(acc.id); setEditing2faValue(""); }}
                      >
                        —
                      </span>
                    )}
                  </td>
                  <td className="px-2 py-3">
                    {!isProcessing && (
                      <span className="inline-flex items-center gap-1">
                        <button
                          onClick={async () => {
                            if (!IS_DEV) await invoke("launch_telegram", { accountId: acc.id });
                          }}
                          className="opacity-0 group-hover:opacity-50 hover:!opacity-100 rounded p-0.5 text-muted-foreground hover:text-primary transition"
                          title={t("accounts.openInTelegram")}
                        >
                          <Send className="h-3 w-3" />
                        </button>
                        <button
                          onClick={() => handleDeleteOne(acc.id)}
                          className="opacity-0 group-hover:opacity-50 hover:!opacity-100 rounded p-0.5 text-muted-foreground hover:text-destructive transition"
                          title={t("accounts.deleteAccount")}
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {sorted.length === 0 && (
          <div className="flex items-center justify-center py-12 text-muted-foreground text-sm">
            {t("accounts.noAccountsFiltered")}
          </div>
        )}
        </div>
      </div>

      {/* pagination */}
      {sorted.length > pageSize && (
        <div className="flex items-center justify-center gap-4">
          <button
            onClick={() => setPage((p) => Math.max(0, p - 1))}
            disabled={page === 0}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm disabled:opacity-30 hover:border-primary/50 transition"
          >
            {t("common.back")}
          </button>
          <span className="text-sm text-muted-foreground">
            {page * pageSize + 1}–{Math.min((page + 1) * pageSize, sorted.length)} / {sorted.length}
          </span>
          <button
            onClick={() => setPage((p) => Math.min(Math.ceil(sorted.length / pageSize) - 1, p + 1))}
            disabled={(page + 1) * pageSize >= sorted.length}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm disabled:opacity-30 hover:border-primary/50 transition"
          >
            {t("common.forward")}
          </button>
        </div>
      )}

      {/* no proxy warning */}
      {noProxyWarning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts.noProxyWarningTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts.noProxyWarningDesc")}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setNoProxyWarning(false)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
              <button
                onClick={() => { setNoProxyWarning(false); setPendingValidateAfterDistribute(true); setPendingValidateIds(Array.from(selected)); setProxyDistributeOpen(true); }}
                className="flex-1 rounded-md border border-primary/50 bg-primary/10 px-3 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition"
              >
                {t("accounts.distributeBtn")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* proxy distribution dialog */}

      {/* threads warning */}
      {threadsWarning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("settings.warning")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts.threadsWarning")}
            </p>
            <button
              onClick={() => setThreadsWarning(false)}
              className="w-full rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
            >
              {t("common.ok")}
            </button>
          </div>
        </div>
      )}
      {proxyDistributeOpen && (
        <ProxyDistributeDialog
          onClose={() => setProxyDistributeOpen(false)}
          onDone={async () => {
            await refreshAccounts();
            if (pendingValidateAfterDistribute) {
              setPendingValidateAfterDistribute(false);
              const ids = pendingValidateIds.length > 0 ? pendingValidateIds : accounts.map(a => a.id);
              setPendingValidateIds([]);
              if (ids.length > 0 && !IS_DEV) {
                setValidating(true);
                setValidatingIds(new Set(ids));
                setSelected(new Set());
                await invoke("enqueue_validate", { ids, checkRestrictions, check2fa, checkAging: true, threads: validateThreads });
                await refreshAccounts();
              }
            }
          }}
        />
      )}

      {/* delete confirmation dialog */}
      {deleteConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-80 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts.deleteConfirmTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts.deleteConfirmDesc", { count: selected.size })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setDeleteConfirm(false)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={handleDeleteAccounts}
                className="flex-1 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive font-medium hover:bg-destructive/20 transition"
              >
                {t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* missing json warning */}
      {missingJsonWarning && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts_import.missingJsonTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-2">
              {t("accounts_import.missingJsonDesc")}
            </p>
            <p className="text-xs text-muted-foreground/60 mb-4">
              {t("accounts_import.missingJsonHint")}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setMissingJsonWarning(false)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* duplicate warning */}
      {duplicateWarning > 0 && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts_import.duplicatesTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts_import.duplicatesDesc", { count: duplicateWarning })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setDuplicateWarning(0)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* multi-account tdata warning */}
      {multiAccountWarning > 0 && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts_import.multiAccountTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts_import.multiAccountDesc", { count: multiAccountWarning })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setMultiAccountWarning(0)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* local passcode warning */}
      {passcodeWarning > 0 && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts_import.passcodeTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts_import.passcodeDesc", { count: passcodeWarning })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setPasscodeWarning(0)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* validate results */}
      {validateResult && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-96 shadow-2xl">
            <h3 className="text-lg font-semibold mb-3">{t("accounts.validateResultTitle")}</h3>
            <div className="text-sm space-y-2 mb-4">
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("accounts.noRestrictions")}:</span>
                <span className="font-medium text-[oklch(0.65_0.1_150)]">{validateResult.valid}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("accounts.withRestrictions")}:</span>
                <span className="font-medium text-[oklch(0.6_0.1_60)]">
                  {validateResult.restricted}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("common.invalid")}:</span>
                <span className="font-medium text-[oklch(0.55_0.1_25)]">{validateResult.invalid}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-muted-foreground">{t("accounts.unreachable")}:</span>
                <span className="font-medium text-[oklch(0.6_0.05_300)]">{validateResult.unreachable}</span>
              </div>
              {validateResult.errors.length > 0 && (
                <div className="mt-2 pt-2 border-t border-border">
                  <div className="text-xs text-muted-foreground mb-1">{t("common.errors")}:</div>
                  {validateResult.errors.map((e, i) => (
                    <div key={i} className="text-xs text-muted-foreground/80">{e}</div>
                  ))}
                </div>
              )}
            </div>
            <div className="flex flex-col gap-2">
              <button
                onClick={() => setValidateResult(null)}
                className="w-full rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.ok")}
              </button>
              {validateResult.invalid > 0 && (
                <button
                  onClick={async () => {
                    if (!IS_DEV) {
                      const ids = accounts.filter((a) => a.status === "Невалид" || a.status === "Invalid").map((a) => a.id);
                      await invoke("delete_accounts", { ids });
                      await refreshAccounts();
                    }
                    setValidateResult(null);
                  }}
                  className="w-full rounded-md border border-border px-3 py-2 text-sm text-muted-foreground font-medium hover:text-destructive hover:border-destructive/30 transition"
                >
                  {t("accounts.deleteInvalid")}
                </button>
              )}
              {(validateResult.invalid > 0 || accounts.some((a) => a.status === "Заморожен" || a.status === "Frozen")) && (
                <button
                  onClick={async () => {
                    if (!IS_DEV) {
                      const ids = accounts.filter((a) => a.status === "Невалид" || a.status === "Заморожен" || a.status === "Invalid" || a.status === "Frozen").map((a) => a.id);
                      await invoke("delete_accounts", { ids });
                      await refreshAccounts();
                    }
                    setValidateResult(null);
                  }}
                  className="w-full rounded-md border border-border px-3 py-2 text-sm text-muted-foreground font-medium hover:text-destructive hover:border-destructive/30 transition"
                >
                  {t("accounts.deleteInvalidAndFrozen")}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* import format dialog */}
      {importDialogOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={(e) => { if (e.target === e.currentTarget) setImportDialogOpen(false); }}>
          <div className="rounded-xl border border-border bg-card p-6 w-80 shadow-2xl relative">
            <button onClick={() => setImportDialogOpen(false)} className="absolute top-3 right-3 text-muted-foreground hover:text-foreground text-lg leading-none">✕</button>
            <h3 className="text-lg font-semibold mb-4">{t("accounts.importBtn")}</h3>
            <div className="space-y-2">
              {[
                { id: "telethon", label: "Telethon (.session)" },
                { id: "tdata", label: "TData (folder)" },
                { id: "tdata_zip", label: "TData (zip)" },
                { id: "pyrogram", label: "Pyrogram (.session)" },
              ].map((f) => (
                <button
                  key={f.id}
                  onClick={() => handleImport(f.id)}
                  className="w-full text-left rounded-md border border-border bg-background px-4 py-3 text-sm font-medium hover:border-primary/50 transition"
                >
                  {f.label}
                </button>
              ))}
              <button
                onClick={() => { setImportDialogOpen(false); setAuthKeyDialogOpen(true); }}
                className="w-full text-left rounded-md border border-border bg-background px-4 py-3 text-sm font-medium hover:border-primary/50 transition"
              >
                Auth Key + DC ID
              </button>
              <button
                onClick={() => { setImportDialogOpen(false); setPhoneLoginOpen(true); }}
                className="w-full text-left rounded-md border border-border bg-background px-4 py-3 text-sm font-medium hover:border-primary/50 transition"
              >
                Phone + code
              </button>
            </div>
          </div>
        </div>
      )}

      {/* auth key import dialog — opens txt in editor, monitors for keys */}
      {authKeyDialogOpen && (
        <AuthKeyImportModal
          onClose={() => setAuthKeyDialogOpen(false)}
          onImported={async () => {
            setAuthKeyDialogOpen(false);
            await refreshAccounts();
          }}
        />
      )}

      {/* phone+code login modal */}
      {phoneLoginOpen && (
        <PhoneLoginModal
          onClose={() => setPhoneLoginOpen(false)}
          onSuccess={async () => {
            setPhoneLoginOpen(false);
            await refreshAccounts();
          }}
        />
      )}

      {/* reauth results modal */}
      {reauthResults && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setReauthResults(null)}>
          <div className="rounded-xl border border-border bg-card p-6 w-[420px] shadow-2xl relative" onClick={(e) => e.stopPropagation()}>
            <button onClick={() => setReauthResults(null)} className="absolute top-3 right-3 text-muted-foreground hover:text-foreground text-lg leading-none">✕</button>
            <h3 className="text-lg font-semibold mb-4">{t("accounts.reauthResultTitle")}</h3>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between"><span className="text-muted-foreground">{t("accounts.reauthSuccess")}:</span><span className="text-[oklch(0.65_0.1_150)] font-medium">{reauthResults.success}</span></div>
              <div className="flex justify-between"><span className="text-muted-foreground">{t("accounts.reauthUnknown2fa")}:</span><span className="text-amber-400 font-medium">{reauthResults.unknown_2fa}</span></div>
              <div className="flex justify-between"><span className="text-muted-foreground">{t("accounts.reauthFailed")}:</span><span className="text-destructive font-medium">{reauthResults.failed}</span></div>
            </div>
            {reauthResults.errors.length > 0 && (
              <div className="mt-4">
                <div className="text-xs text-muted-foreground mb-1">{t("common.errors")}:</div>
                <div className="rounded-md border border-border bg-muted/30 px-3 py-2 max-h-32 overflow-y-auto text-xs space-y-1">
                  {Object.entries(reauthResults.errors.reduce<Record<string, number>>((acc, e) => { acc[e] = (acc[e] || 0) + 1; return acc; }, {}))
                    .sort((a, b) => b[1] - a[1])
                    .slice(0, 10)
                    .map(([err, count]) => (
                      <div key={err} className="flex justify-between gap-2">
                        <span className="text-muted-foreground truncate">{err}</span>
                        <span className="text-foreground font-medium shrink-0">{count}</span>
                      </div>
                    ))}
                </div>
              </div>
            )}
            <button onClick={() => setReauthResults(null)} className="mt-4 w-full rounded-md border border-border px-3 py-2 text-sm font-medium hover:bg-accent/50 transition">
              {t("common.close")}
            </button>
          </div>
        </div>
      )}

      {/* single account delete confirmation */}
      {deleteOneId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="rounded-xl border border-border bg-card p-6 w-80 shadow-2xl">
            <h3 className="text-lg font-semibold mb-2">{t("accounts.deleteConfirmTitle")}</h3>
            <p className="text-sm text-muted-foreground mb-4">
              {t("accounts.deleteConfirmDesc", { count: 1 })}
            </p>
            <div className="flex gap-2">
              <button
                onClick={() => setDeleteOneId(null)}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm hover:bg-accent/50 transition"
              >
                {t("common.cancel")}
              </button>
              <button
                onClick={confirmDeleteOne}
                className="flex-1 rounded-md border border-border px-3 py-2 text-sm text-muted-foreground font-medium hover:text-destructive hover:border-destructive/30 transition"
              >
                Удалить
              </button>
            </div>
          </div>
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
