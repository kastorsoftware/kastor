import { useState, useEffect, useMemo, lazy, Suspense } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Send, ShieldCheck, Users, UserPlus, MessageSquare,
  Activity, Settings as SettingsIcon,
  Cpu, Wifi, Construction, Repeat, UserCog, Flag, PlusCircle, Radio, TrendingUp, Copy, Download, Shuffle, AtSign,
  Eye, Upload, Zap, Link2, Search, Forward, ChevronDown, Bot, Github,
} from "lucide-react";
import { useI18n, useT } from "@/i18n";

// lazy-load pages so each becomes its own chunk
const AccountsPage = lazy(() => import("@/components/AccountsPage").then(m => ({ default: m.AccountsPage })));
const ProxySettings = lazy(() => import("@/components/ProxySettings").then(m => ({ default: m.ProxySettings })));
const SettingsPage = lazy(() => import("@/components/SettingsPage").then(m => ({ default: m.SettingsPage })));
const CheckerPage = lazy(() => import("@/components/CheckerPage").then(m => ({ default: m.CheckerPage })));
const ConverterPage = lazy(() => import("@/components/ConverterPage").then(m => ({ default: m.ConverterPage })));
const AccountActionsPage = lazy(() => import("@/components/AccountActionsPage").then(m => ({ default: m.AccountActionsPage })));
const ReporterPage = lazy(() => import("@/components/ReporterPage").then(m => ({ default: m.ReporterPage })));
const CreateBotsPage = lazy(() => import("@/components/CreateBotsPage").then(m => ({ default: m.CreateBotsPage })));
const CreateChannelsPage = lazy(() => import("@/components/CreateChannelsPage").then(m => ({ default: m.CreateChannelsPage })));
const BoostPage = lazy(() => import("@/components/BoostPage").then(m => ({ default: m.BoostPage })));
const ClonerPage = lazy(() => import("@/components/ClonerPage").then(m => ({ default: m.ClonerPage })));
const ParserPage = lazy(() => import("@/components/ParserPage").then(m => ({ default: m.ParserPage })));
const RandomizerPage = lazy(() => import("@/components/RandomizerPage").then(m => ({ default: m.RandomizerPage })));
const UsernameCheckerPage = lazy(() => import("@/components/UsernameCheckerPage").then(m => ({ default: m.UsernameCheckerPage })));
const LinkCheckerPage = lazy(() => import("@/components/LinkCheckerPage").then(m => ({ default: m.LinkCheckerPage })));
const GlobalSearchPage = lazy(() => import("@/components/GlobalSearchPage").then(m => ({ default: m.GlobalSearchPage })));
const ForwarderPage = lazy(() => import("@/components/ForwarderPage").then(m => ({ default: m.ForwarderPage })));
const WarmerPage = lazy(() => import("@/components/WarmerPage").then(m => ({ default: m.WarmerPage })));
const BotParserPage = lazy(() => import("@/components/BotParserPage").then(m => ({ default: m.BotParserPage })));
const StoriesPage = lazy(() => import("@/components/StoriesPage").then(m => ({ default: m.StoriesPage })));
const AutoReplyPage = lazy(() => import("@/components/AutoReplyPage").then(m => ({ default: m.AutoReplyPage })));
const FirstCommentPage = lazy(() => import("@/components/FirstCommentPage").then(m => ({ default: m.FirstCommentPage })));
const InviterPage = lazy(() => import("@/components/InviterPage").then(m => ({ default: m.InviterPage })));
const InterceptorPage = lazy(() => import("@/components/InterceptorPage").then(m => ({ default: m.InterceptorPage })));
const MasslookingPage = lazy(() => import("@/components/MasslookingPage").then(m => ({ default: m.MasslookingPage })));
const MailingPage = lazy(() => import("@/components/MailingPage").then(m => ({ default: m.MailingPage })));

interface DashboardStats {
  accounts: number;
  messages_today: number;
  queue: number;
  proxies: number;
}

type NavKey =
  | "dashboard" | "proxies" | "accounts" | "checker" | "account-actions" | "converter"
  | "warmer" | "mailing" | "inviter" | "interceptor" | "auto-reply"
  | "reporter" | "create-bots" | "create-channels" | "boost" | "cloner" | "parser"
  | "randomizer" | "username-checker" | "link-checker" | "global-search" | "forwarder" | "masslooking" | "stories" | "bot-parser"
  | "firstnakh"
  | "settings";

export function Dashboard() {
  const t = useT();
  const [active, setActive] = useState<NavKey>("dashboard");
  const [visited, setVisited] = useState<Set<NavKey>>(() => new Set(["dashboard"]));
  const [version, setVersion] = useState("");
  const [stats, setStats] = useState<DashboardStats | null>(null);

  const navItems: { key: NavKey; label: string; icon: typeof Send }[] = [
    { key: "dashboard", label: t("nav.dashboard"), icon: Activity },
    { key: "proxies", label: t("nav.proxy"), icon: Wifi },
    { key: "accounts", label: t("nav.accounts"), icon: Users },
    { key: "checker", label: t("nav.checker"), icon: ShieldCheck },
    { key: "account-actions", label: t("nav.accountActions"), icon: UserCog },
    { key: "converter", label: t("nav.converter"), icon: Repeat },
    { key: "warmer", label: t("nav.warmer"), icon: Activity },
    { key: "bot-parser", label: t("nav.botParser"), icon: Bot },
    { key: "mailing", label: t("nav.mailing"), icon: Send },
    { key: "inviter", label: t("nav.inviter"), icon: UserPlus },
    { key: "interceptor", label: t("nav.interceptor"), icon: Radio },
    { key: "auto-reply", label: t("nav.autoReply"), icon: MessageSquare },
    { key: "boost", label: t("nav.boost"), icon: TrendingUp },
    { key: "masslooking", label: t("nav.masslooking"), icon: Eye },
    { key: "stories", label: t("nav.stories"), icon: Upload },
    { key: "reporter", label: t("nav.reporter"), icon: Flag },
    { key: "create-bots", label: t("nav.createBots"), icon: PlusCircle },
    { key: "create-channels", label: t("nav.createChannels"), icon: Radio },
    { key: "cloner", label: t("nav.cloner"), icon: Copy },
    { key: "parser", label: t("nav.parser"), icon: Download },
    { key: "randomizer", label: t("nav.randomizer"), icon: Shuffle },
    { key: "username-checker", label: t("nav.usernameChecker"), icon: AtSign },
    { key: "link-checker", label: t("nav.linkChecker"), icon: Link2 },
    { key: "global-search", label: t("nav.globalSearch"), icon: Search },
    { key: "forwarder", label: t("nav.forwarder"), icon: Forward },
    { key: "firstnakh", label: t("nav.firstComment"), icon: Zap },
  ];

  // Top-level items (always visible, not in any category)
  const topItems = navItems.filter(item => ["dashboard", "proxies", "accounts"].includes(item.key));

  // Category definitions
  const categories: { id: string; label: string; icon: typeof Send; items: NavKey[] }[] = [
    { id: "accounts-cat", label: t("filters.catAccounts"), icon: UserCog, items: ["checker", "account-actions", "converter"] },
    { id: "mailing-cat", label: t("filters.catMailing"), icon: Send, items: ["mailing", "auto-reply", "firstnakh"] },
    { id: "promo-cat", label: t("filters.catPromo"), icon: TrendingUp, items: ["inviter", "boost", "masslooking", "stories"] },
    { id: "parser-cat", label: t("filters.catParser"), icon: Download, items: ["parser", "global-search", "username-checker", "link-checker"] },
    { id: "tools-cat", label: t("filters.catTools"), icon: Construction, items: ["warmer", "bot-parser", "cloner", "create-bots", "create-channels", "randomizer", "interceptor", "forwarder", "reporter"] },
  ];

  // Determine which category contains the active item (for auto-expand)
  const activeCategoryId = useMemo(() => {
    for (const cat of categories) {
      if (cat.items.includes(active)) return cat.id;
    }
    return null;
  }, [active]);

  const [openCategories, setOpenCategories] = useState<Set<string>>(() => {
    // Start with the category containing the active item open
    const initial = new Set<string>();
    for (const cat of categories) {
      if (cat.items.includes("dashboard")) initial.add(cat.id);
    }
    return initial;
  });

  // Auto-expand category when active item changes
  useEffect(() => {
    if (activeCategoryId && !openCategories.has(activeCategoryId)) {
      setOpenCategories(prev => {
        const next = new Set(prev);
        next.add(activeCategoryId);
        return next;
      });
    }
  }, [activeCategoryId]);

  const toggleCategory = (id: string) => {
    setOpenCategories(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const settingsLabel = t("nav.settings");

  // mark a tab as visited (first time only) so its chunk is loaded and component stays mounted
  const navigate = (key: NavKey) => {
    setActive(key);
    if (!visited.has(key)) {
      setVisited(prev => {
        const next = new Set(prev);
        next.add(key);
        return next;
      });
    }
  };

  useEffect(() => {
    const IS_DEV = !("__TAURI_INTERNALS__" in window);
    if (IS_DEV) {
      setVersion("0.1");
      setStats({ accounts: 0, messages_today: 0, queue: 0, proxies: 0 });
      return;
    }
    invoke<string>("get_version").then(setVersion);
    invoke<DashboardStats>("get_stats").then(setStats);

    // refresh stats periodically
    const interval = setInterval(() => {
      invoke<DashboardStats>("get_stats").then(setStats);
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const renderContent = () => {
    return (
      <Suspense fallback={<PageFallback />}>
        <div style={{ display: active === "dashboard" ? "block" : "none" }}>
          {stats ? <DashboardHome stats={stats} /> : null}
        </div>
        {visited.has("proxies") && (
          <div style={{ display: active === "proxies" ? "block" : "none" }}>
            <ProxySettings />
          </div>
        )}
        {visited.has("accounts") && (
          <div style={{ display: active === "accounts" ? "block" : "none" }}>
            <AccountsPage />
          </div>
        )}
        {visited.has("checker") && (
          <div style={{ display: active === "checker" ? "block" : "none" }}>
            <CheckerPage />
          </div>
        )}
        {visited.has("account-actions") && (
          <div style={{ display: active === "account-actions" ? "block" : "none" }}>
            <AccountActionsPage />
          </div>
        )}
        {visited.has("converter") && (
          <div style={{ display: active === "converter" ? "block" : "none" }}>
            <ConverterPage />
          </div>
        )}
        {visited.has("reporter") && (
          <div style={{ display: active === "reporter" ? "block" : "none" }}>
            <ReporterPage />
          </div>
        )}
        {visited.has("create-bots") && (
          <div style={{ display: active === "create-bots" ? "block" : "none" }}>
            <CreateBotsPage />
          </div>
        )}
        {visited.has("create-channels") && (
          <div style={{ display: active === "create-channels" ? "block" : "none" }}>
            <CreateChannelsPage />
          </div>
        )}
        {visited.has("boost") && (
          <div style={{ display: active === "boost" ? "block" : "none" }}>
            <BoostPage />
          </div>
        )}
        {visited.has("stories") && (
          <div style={{ display: active === "stories" ? "block" : "none" }}>
            <StoriesPage />
          </div>
        )}
        {visited.has("auto-reply") && (
          <div style={{ display: active === "auto-reply" ? "block" : "none" }}>
            <AutoReplyPage />
          </div>
        )}
        {visited.has("firstnakh") && (
          <div style={{ display: active === "firstnakh" ? "block" : "none" }}>
            <FirstCommentPage />
          </div>
        )}
        {visited.has("inviter") && (
          <div style={{ display: active === "inviter" ? "block" : "none" }}>
            <InviterPage />
          </div>
        )}
        {visited.has("interceptor") && (
          <div style={{ display: active === "interceptor" ? "block" : "none" }}>
            <InterceptorPage />
          </div>
        )}
        {visited.has("masslooking") && (
          <div style={{ display: active === "masslooking" ? "block" : "none" }}>
            <MasslookingPage />
          </div>
        )}
        {visited.has("mailing") && (
          <div style={{ display: active === "mailing" ? "block" : "none" }}>
            <MailingPage />
          </div>
        )}
        {visited.has("cloner") && (
          <div style={{ display: active === "cloner" ? "block" : "none" }}>
            <ClonerPage />
          </div>
        )}
        {visited.has("parser") && (
          <div style={{ display: active === "parser" ? "block" : "none" }}>
            <ParserPage />
          </div>
        )}
        {visited.has("randomizer") && (
          <div style={{ display: active === "randomizer" ? "block" : "none" }}>
            <RandomizerPage />
          </div>
        )}
        {visited.has("username-checker") && (
          <div style={{ display: active === "username-checker" ? "block" : "none" }}>
            <UsernameCheckerPage />
          </div>
        )}
        {visited.has("link-checker") && (
          <div style={{ display: active === "link-checker" ? "block" : "none" }}>
            <LinkCheckerPage />
          </div>
        )}
        {visited.has("global-search") && (
          <div style={{ display: active === "global-search" ? "block" : "none" }}>
            <GlobalSearchPage />
          </div>
        )}
        {visited.has("forwarder") && (
          <div style={{ display: active === "forwarder" ? "block" : "none" }}>
            <ForwarderPage />
          </div>
        )}
        {visited.has("warmer") && (
          <div style={{ display: active === "warmer" ? "block" : "none" }}>
            <WarmerPage />
          </div>
        )}
        {visited.has("bot-parser") && (
          <div style={{ display: active === "bot-parser" ? "block" : "none" }}>
            <BotParserPage />
          </div>
        )}
        {active === "settings" && <SettingsPage />}
        {active !== "dashboard" && active !== "proxies" && active !== "accounts" && active !== "checker" && active !== "account-actions" && active !== "converter" && active !== "reporter" && active !== "create-bots" && active !== "create-channels" && active !== "boost" && active !== "cloner" && active !== "parser" && active !== "randomizer" && active !== "username-checker" && active !== "link-checker" && active !== "global-search" && active !== "forwarder" && active !== "warmer" && active !== "bot-parser" && active !== "settings" && active !== "stories" && active !== "auto-reply" && active !== "firstnakh" && active !== "inviter" && active !== "interceptor" && active !== "masslooking" && active !== "mailing" && (
          <Placeholder section={navItems.find((n) => n.key === active)?.label || ""} />
        )}
      </Suspense>
    );
  };

  return (
    <div className="flex h-screen bg-background text-foreground">
      <aside className="flex w-64 flex-col border-r border-border bg-sidebar h-screen">
        <div className="px-6 py-5 border-b border-sidebar-border">
          <button
            onClick={() => invoke("open_url", { url: "https://github.com/kastorsoftware/kastor" })}
            className="mb-2 flex h-8 w-8 items-center justify-center rounded-md border border-sidebar-border text-muted-foreground transition hover:border-primary/50 hover:text-sidebar-foreground"
            title="GitHub"
            aria-label="Open Kastor on GitHub"
          >
            <Github className="h-4 w-4" />
          </button>
          <div>
            <div className="text-base font-bold text-sidebar-foreground">Kastor</div>
            <div className="text-[10px] uppercase tracking-wider text-muted-foreground">
              v{version}
            </div>
          </div>
        </div>

        <nav className="flex-1 px-3 py-4 space-y-1 overflow-y-auto scrollbar-thin">
          {/* Top-level items */}
          {topItems.map((item) => (
            <NavBtn key={item.key} item={item} active={active === item.key} onClick={() => navigate(item.key)} />
          ))}

          {/* Categories */}
          {categories.map((cat) => {
            const isOpen = openCategories.has(cat.id);
            const CatIcon = cat.icon;
            const catNavItems = cat.items.map(key => navItems.find(n => n.key === key)!).filter(Boolean);
            return (
              <div key={cat.id} className="pt-2">
                <button
                  onClick={() => toggleCategory(cat.id)}
                  className="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-[13px] font-semibold text-sidebar-foreground/80 hover:bg-sidebar-accent/50 transition"
                >
                  <CatIcon className="h-3.5 w-3.5" />
                  <span className="flex-1 text-left">{cat.label}</span>
                  <ChevronDown className={`h-3.5 w-3.5 transition-transform ${isOpen ? "" : "-rotate-90"}`} />
                </button>
                {isOpen && (
                  <div className="mt-0.5 space-y-0.5">
                    {catNavItems.map((item) => (
                      <NavBtn key={item.key} item={item} active={active === item.key} onClick={() => navigate(item.key)} indent />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </nav>

        <div className="px-3 py-3 border-t border-sidebar-border space-y-1">
          <div
            className="flex items-center gap-3 rounded-md px-3 py-2.5 cursor-pointer transition hover:bg-sidebar-accent"
            onClick={() => navigate("settings")}
          >
            <div
              className="flex h-9 w-9 items-center justify-center rounded-full text-sm font-bold text-primary-foreground"
              style={{ background: "var(--gradient-primary)" }}
            >
              K
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-semibold text-sidebar-foreground truncate">Kastor</div>
            </div>
            <SettingsIcon className="h-4 w-4 text-muted-foreground shrink-0" />
          </div>
        </div>
      </aside>

      <main className="flex-1 min-w-0 overflow-auto">
        <header className="flex items-center justify-between border-b border-border px-8 py-4">
          <h1 className="text-2xl font-bold">
            {active === "settings" ? settingsLabel : navItems.find((n) => n.key === active)?.label}
          </h1>
          <div className="flex items-center gap-3">
            <LanguageToggle />
            <button
              onClick={() => navigate("settings")}
              className={`flex h-9 w-9 items-center justify-center rounded-md border transition ${
                active === "settings"
                  ? "border-primary/50 bg-accent/40 text-primary"
                  : "border-border bg-card text-muted-foreground hover:border-primary/50 hover:text-foreground"
              }`}
              aria-label={settingsLabel}
            >
              <SettingsIcon className="h-4 w-4" />
            </button>
          </div>
        </header>

        <div className="p-8">
          {renderContent()}
        </div>
      </main>
    </div>
  );
}

function NavBtn({
  item, active, onClick, indent,
}: { item: { label: string; icon: typeof Send }; active: boolean; onClick: () => void; indent?: boolean }) {
  const Icon = item.icon;
  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-2.5 rounded-md ${indent ? "pl-5 pr-2.5" : "px-2.5"} py-2 text-[13px] transition ${
        active
          ? "bg-sidebar-accent text-sidebar-accent-foreground shadow-sm border border-primary/30 font-semibold"
          : "text-sidebar-foreground hover:bg-sidebar-accent/50 font-medium"
      }`}
      style={active ? { boxShadow: "inset 0 0 0 1px oklch(0.62 0.24 300 / 0.3)" } : undefined}
    >
      <Icon className={`h-3.5 w-3.5 ${active ? "text-primary" : ""}`} />
      {item.label}
    </button>
  );
}

function DashboardHome({ stats }: { stats: DashboardStats }) {
  const t = useT();
  const [tasks, setTasks] = useState<{ id: string; kind: string; description: string; status: string }[]>([]);
  useEffect(() => {
    const IS_DEV = !("__TAURI_INTERNALS__" in window);
    if (IS_DEV) return;
    const load = () => invoke<{ id: string; kind: string; description: string; status: string }[]>("get_task_queue").then(setTasks);
    load();
    const interval = setInterval(load, 3000);
    return () => clearInterval(interval);
  }, []);

  const localizeKind = (kind: string): string => {
    const map: Record<string, string> = {
      "mailing": t("nav.mailing"),
      "inviter": t("nav.inviter"),
      "checker": t("nav.checker"),
      "warmer": t("nav.warmer"),
      "bot_parser": t("nav.botParser"),
      "parser": t("nav.parser"),
      "boost": t("nav.boost"),
      "masslooking": t("nav.masslooking"),
      "converter": t("nav.converter"),
      "auto_reply": t("nav.autoReply"),
      "first_comment": t("nav.firstComment"),
      "reporter": t("nav.reporter"),
      "channelcreator": t("nav.createChannels"),
      "botcreator": t("nav.createBots"),
      "stories": t("nav.stories"),
      "cloner": t("nav.cloner"),
      "interceptor": t("nav.interceptor"),
      "randomizer": t("nav.randomizer"),
      "username_checker": t("nav.usernameChecker"),
      "global_search": t("nav.globalSearch"),
      "account_actions": t("nav.accountActions"),
      "link_checker": t("nav.linkChecker"),
      "validate": t("accounts.validation"),
      "user_lookup": t("parser.modeUserLookup"),
      "forwarder": t("nav.forwarder"),
    };
    return map[kind] || kind;
  };

  const statusColor = (s: string) => {
    switch (s) {
      case "Done": return "oklch(0.65 0.1 150)";
      case "Running": return "oklch(0.65 0.1 280)";
      case "Queued": return "oklch(0.6 0.1 60)";
      case "Failed": return "oklch(0.55 0.1 25)";
      default: return "oklch(0.6 0.05 300)";
    }
  };

  const statusLabel = (s: string) => {
    switch (s) {
      case "Done": return t("dashboard.statusDone");
      case "Running": return t("dashboard.statusRunning");
      case "Queued": return t("dashboard.statusQueued");
      case "Failed": return t("dashboard.statusFailed");
      default: return s;
    }
  };

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-4">
        <StatCard icon={Users} label={t("common.accounts")} value={stats.accounts.toLocaleString()} />
        <StatCard icon={Send} label={t("dashboard.messagesToday")} value={stats.messages_today.toLocaleString()} />
        <StatCard icon={Cpu} label={t("dashboard.queue")} value={stats.queue.toLocaleString()} />
        <StatCard icon={Wifi} label={t("common.proxies")} value={stats.proxies.toLocaleString()} />
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="rounded-xl border border-border bg-card p-6 flex flex-col">
          <h2 className="text-sm font-semibold mb-4">{t("dashboard.recentTasks")}</h2>
          <div className="space-y-3 flex-1">
            {tasks.length === 0 && (
              <div className="text-sm text-muted-foreground py-4 text-center">{t("dashboard.noTasks")}</div>
            )}
            {tasks.slice(-4).reverse().map((task) => (
              <div key={task.id} className="flex items-center justify-between rounded-md border border-border bg-background/40 px-4 py-3">
                <div>
                  <div className="text-sm font-medium">{task.description}</div>
                  <div className="text-xs text-muted-foreground">{localizeKind(task.kind)}</div>
                </div>
                <StatusBadge label={statusLabel(task.status)} color={statusColor(task.status)} />
              </div>
            ))}
          </div>
        </div>

      </div>
    </div>
  );
}

function StatusBadge({ label, color }: { label: string; color: string }) {
  return (
    <span
      className="inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium"
      style={{
        color,
        borderColor: `color-mix(in oklch, ${color} 45%, transparent)`,
        background: `color-mix(in oklch, ${color} 10%, transparent)`,
      }}
    >
      <span
        className="h-2 w-2 rounded-full"
        style={{
          background: color,
          boxShadow: `0 0 0 2px color-mix(in oklch, ${color} 30%, transparent)`,
        }}
      />
      {label}
    </span>
  );
}

function StatCard({ icon: Icon, label, value, sub }: { icon: typeof Send; label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-xl border border-border bg-card p-5 transition hover:border-primary/40">
      <div className="flex items-center justify-between">
        <span className="text-xs text-muted-foreground">{label}</span>
        <Icon className="h-4 w-4 text-primary" />
      </div>
      <div className="mt-3 text-4xl font-bold tracking-tight">{value}</div>
      {sub && <div className="mt-1 text-xs text-muted-foreground">{sub}</div>}
    </div>
  );
}

function Placeholder({ section }: { section: string }) {
  const t = useT();
  return (
    <div className="flex h-[75vh] flex-col items-center justify-center rounded-xl border border-dashed border-border bg-card/50">
      <Construction className="h-12 w-12 text-muted-foreground mb-4" />
      <h2 className="text-xl font-semibold">{t("dashboard.moduleTitle", { name: section })}</h2>
      <p className="mt-2 text-sm text-muted-foreground">{t("dashboard.inDevelopment")}</p>
    </div>
  );
}

function LanguageToggle() {
  const { locale, setLocale } = useI18n();
  return (
    <button
      onClick={() => setLocale(locale === "ru" ? "en" : "ru")}
      className="flex h-9 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 text-sm font-medium text-muted-foreground hover:border-primary/50 hover:text-foreground transition"
      title={locale === "ru" ? "Switch to English" : "Переключить на русский"}
    >
      {locale === "ru" ? "Ru" : "En"}
    </button>
  );
}

function PageFallback() {
  return (
    <div className="flex h-[60vh] items-center justify-center">
      <div className="h-8 w-8 animate-spin rounded-full border-2 border-primary/30 border-t-primary" />
    </div>
  );
}
