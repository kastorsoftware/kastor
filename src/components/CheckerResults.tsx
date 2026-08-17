import { useState, useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X, ArrowLeft, FolderDown } from "lucide-react";
import { useT } from "@/i18n";
import { PieChart as RechartsPieChart, Pie, Cell, Tooltip } from "recharts";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogAction,
  AlertDialogCancel,
} from "@/components/ui/alert-dialog";
import type { CheckerAccount } from "@/components/CheckerPage";

interface CheckerResultsProps {
  open: boolean;
  onClose: () => void;
  accounts: CheckerAccount[];
  invalidCount: number;
  enabledChecks: {
    twoFa: boolean;
    stars: boolean;
    channels: boolean;
    groups: boolean;
    shortTag: boolean;
    shortChannelTag: boolean;
    nftGifts: boolean;
    premium: boolean;
    cryptoBots: boolean;
    seedPhrases: boolean;
    passFiles: boolean;
    channelCount: boolean;
    phone888: boolean;
    nftTags: boolean;
    shortId: boolean;
    regDate: boolean;
    channelBalances: boolean;
  };
}

const PIE_COLORS = {
  invalid: "#dc2626",
  noRestrictions: "#16a34a",
  tempSpamblock: "#ca8a04",
  permSpamblock: "#ea580c",
  frozen: "#2563eb",
} as const;

const LEGEND = [
  { key: "invalid", label: "checker.noResults", color: PIE_COLORS.invalid },
  { key: "noRestrictions", label: "checker.noRestrictions", color: PIE_COLORS.noRestrictions },
  { key: "tempSpamblock", label: "checker.tempSpamblock", color: PIE_COLORS.tempSpamblock },
  { key: "permSpamblock", label: "checker.permSpamblock", color: PIE_COLORS.permSpamblock },
  { key: "frozen", label: "checker.frozen", color: PIE_COLORS.frozen },
] as const;

const COUNTRY_COLORS = ["#8b5cf6", "#06b6d4", "#f59e0b", "#ec4899", "#10b981", "#6366f1", "#6b7280"];

function PieChart({ data }: { data: { value: number; color: string; name?: string }[] }) {
  const t = useT();
  const filtered = data.filter((d) => d.value > 0);
  if (filtered.length === 0) {
    return (
      <div className="w-56 h-56 rounded-lg border border-border flex items-center justify-center">
        <span className="text-muted-foreground text-sm">{t("common.noData")}</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border p-3">
      <RechartsPieChart width={200} height={200}>
        <Pie
          data={filtered}
          dataKey="value"
          nameKey="name"
          cx="50%"
          cy="50%"
          outerRadius={90}
          strokeWidth={0}
        >
          {filtered.map((entry, i) => (
            <Cell key={i} fill={entry.color} />
          ))}
        </Pie>
        <Tooltip
          content={({ payload }) => {
            if (!payload || !payload.length) return null;
            const d = payload[0].payload;
            return (
              <div className="rounded border border-border bg-card px-2 py-1 text-xs shadow">
                {d.name || "—"}: {d.value}
              </div>
            );
          }}
        />
      </RechartsPieChart>
    </div>
  );
}

type SubMenuType =
  | "channels"
  | "groups"
  | "nft_gifts"
  | "nft_tags"
  | "phone888"
  | "premium"
  | "reg_date"
  | "seed"
  | "pass_files"
  | "stars"
  | "crypto_bots"
  | "channel_balances"
  | null;

export function CheckerResults({ open, onClose, accounts, invalidCount, enabledChecks }: CheckerResultsProps) {
  const t = useT();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [activeSubMenu, setActiveSubMenu] = useState<SubMenuType>(null);
  const [sorting, setSorting] = useState(false);
  const [saved, setSaved] = useState(false);

  const countryMap = useCountryMap(accounts);
  const countryGroups = useCountryGroups(accounts.filter((a) => a.spamblock === "none" || a.spamblock === "temp_geo" || a.spamblock === "perm"), countryMap);
  useNftPreloader(accounts);

  if (!open) return null;

  const noRestrictions = accounts.filter((a) => a.spamblock === "none").length;
  const tempSpamblock = accounts.filter((a) => a.spamblock === "temp_geo").length;
  const permSpamblock = accounts.filter((a) => a.spamblock === "perm").length;
  const frozen = accounts.filter((a) => a.spamblock === "frozen").length;

  const withChannels = accounts.filter((a) => a.channels.length > 0);
  const withGroups = accounts.filter((a) => a.groups.length > 0);
  const withNftGifts = accounts.filter((a) => a.nft_gifts.length > 0);
  const withNftTags = accounts.filter((a) => a.nft_tags.length > 0);
  const with888 = accounts.filter((a) => a.phone888);
  const withPremium = accounts.filter((a) => a.premium);
  const withRegDate = accounts.filter((a) => a.reg_date !== null);
  const withSeed = accounts.filter((a) => a.seed_found);
  const withPassFiles = accounts.filter((a) => a.pass_files.length > 0);
  const with2fa = accounts.filter((a) => a.has_2fa).length;
  const withStars = accounts.filter((a) => a.stars > 0);
  const withCryptoBots = accounts.filter((a) => a.crypto_bots.send || a.crypto_bots.xrocket);
  const withChannelBalances = accounts.filter((a) => a.channel_balances && a.channel_balances.length > 0);

  const pieData = [
    { value: invalidCount, color: PIE_COLORS.invalid, name: t("common.invalid") },
    { value: noRestrictions, color: PIE_COLORS.noRestrictions, name: t("checker.noRestrictions") },
    { value: tempSpamblock, color: PIE_COLORS.tempSpamblock, name: t("checker.tempSpamblock") },
    { value: permSpamblock, color: PIE_COLORS.permSpamblock, name: t("checker.permSpamblock") },
    { value: frozen, color: PIE_COLORS.frozen, name: t("checker.frozen") },
  ];

  const tryClose = () => {
    if (saved) { onClose(); return; }
    setConfirmOpen(true);
  };
  const handleConfirmClose = () => { setConfirmOpen(false); onClose(); };

  const handleBackdropClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) {
      e.stopPropagation();
      tryClose();
    }
  };

  const handleSort = async () => {
    setSorting(true);
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const selected = await openDialog({ directory: true, multiple: false, title: t("checker.sortBtn") });
      if (selected) {
        await invoke("checker_sort_results", { accounts, destPath: selected });
        setSaved(true);
      }
    } catch (e) {
      console.error(e);
    }
    setSorting(false);
  };

  const statRows: { label: string; count: number; key: SubMenuType; clickable: boolean }[] = [
    ...(enabledChecks.channels ? [{ label: t("checker.withChannels"), count: withChannels.length, key: "channels" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.groups ? [{ label: t("checker.withGroups"), count: withGroups.length, key: "groups" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.nftGifts ? [{ label: t("checker.withNftGifts"), count: withNftGifts.length, key: "nft_gifts" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.nftTags ? [{ label: t("checker.withNftTags"), count: withNftTags.length, key: "nft_tags" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.phone888 ? [{ label: t("checker.with888"), count: with888.length, key: "phone888" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.premium ? [{ label: t("checker.withPremium"), count: withPremium.length, key: "premium" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.regDate ? [{ label: t("checker.withRegDate"), count: withRegDate.length, key: "reg_date" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.seedPhrases ? [{ label: t("checker.withSeed"), count: withSeed.length, key: "seed" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.passFiles ? [{ label: t("checker.withPassFiles"), count: withPassFiles.length, key: "pass_files" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.twoFa ? [{ label: t("checker.with2fa"), count: with2fa, key: null as SubMenuType, clickable: false }] : []),
    ...(enabledChecks.stars ? [{ label: t("checker.withStars"), count: withStars.length, key: "stars" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.cryptoBots ? [{ label: t("checker.withCryptoBots"), count: withCryptoBots.length, key: "crypto_bots" as SubMenuType, clickable: true }] : []),
    ...(enabledChecks.channelBalances ? [{ label: t("checker.withChannelBalances"), count: withChannelBalances.length, key: "channel_balances" as SubMenuType, clickable: true }] : []),
  ];

  return (
    <>
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70" onClick={handleBackdropClick}>
        <div className="relative w-[75vw] max-w-[900px] h-[80vh] rounded-lg border border-border bg-card flex flex-col overflow-hidden">
          {/* header */}
          <div className="flex items-center justify-between px-5 py-3 border-b border-border shrink-0">
            <button
              onClick={tryClose}
              className="rounded-md p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted transition"
            >
              <X className="h-5 w-5" />
            </button>
            <h2 className="text-base font-bold text-foreground">{t("checker.resultsTitle")}</h2>
            <div className="w-8" />
          </div>

          {/* content */}
          <div className="flex-1 overflow-y-auto scrollbar-thin p-5">
            {activeSubMenu ? (
              <SubMenuPanel
                type={activeSubMenu}
                accounts={accounts}
                onBack={() => setActiveSubMenu(null)}
                countryMap={countryMap}
              />
            ) : (
              <div className="flex gap-8 items-start">
                {/* left: pie chart + legend */}
                <div className="flex flex-col items-center gap-3 shrink-0">
                  <PieChart data={pieData} />
                  <div className="flex flex-col gap-1 text-sm">
                    {LEGEND.map((item) => (
                      <div key={item.key} className="flex items-center gap-2">
                        <span className="w-3 h-3 rounded-sm shrink-0" style={{ backgroundColor: item.color }} />
                        <span className="text-muted-foreground">{t(item.label)}</span>
                      </div>
                    ))}
                  </div>
                </div>

                {/* center: stats */}
                <div className="flex flex-col gap-1.5 min-w-0 flex-1">
                  <div className="text-base font-bold text-foreground mb-1">
                    {t("checker.noRestrictions")}: <span style={{ color: PIE_COLORS.noRestrictions }}>{noRestrictions}</span>
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {t("common.invalid")}: <span style={{ color: PIE_COLORS.invalid }}>{invalidCount}</span>
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {t("checker.tempSpamblock")}: <span style={{ color: PIE_COLORS.tempSpamblock }}>{tempSpamblock}</span>
                  </div>
                  <div className="text-sm text-muted-foreground">
                    {t("checker.permSpamblock")}: <span style={{ color: PIE_COLORS.permSpamblock }}>{permSpamblock}</span>
                  </div>
                  <div className="text-sm text-muted-foreground mb-3">
                    {t("checker.frozen")}: <span style={{ color: PIE_COLORS.frozen }}>{frozen}</span>
                  </div>

                  <div className="border-t border-border pt-2 flex flex-col gap-1">
                    {statRows.map((row) => {
                      if (!row.clickable) {
                        return (
                          <div key={row.label} className="text-sm text-muted-foreground py-0.5">
                            {row.label}: <span className="font-medium text-foreground">{row.count}</span>
                          </div>
                        );
                      }
                      return (
                        <button
                          key={row.label}
                          onClick={() => setActiveSubMenu(row.key)}
                          className="text-left text-sm text-muted-foreground hover:text-foreground transition py-0.5 rounded hover:bg-muted/50 px-1.5 -mx-1.5"
                        >
                          {row.label}: <span className="font-medium text-foreground">{row.count > 0 ? row.count : t("checker.noResults")}</span>
                          {row.count > 0 && <span className="text-xs ml-1.5 text-primary">→</span>}
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* right: country breakdown pie */}
                {countryGroups.length > 1 && (
                  <CountryPieChart groups={countryGroups} />
                )}
              </div>
            )}
          </div>

          {/* footer */}
          <div className="flex items-center justify-end px-5 py-3 border-t border-border shrink-0">
            <button
              onClick={handleSort}
              disabled={sorting || accounts.length === 0}
              className="flex items-center gap-2 rounded-md border border-border bg-primary/10 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
            >
              <FolderDown className="h-4 w-4" />
              {sorting ? t("checker.sorting") : t("checker.sortBtn")}
            </button>
          </div>
        </div>
      </div>

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("checker.closeConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("checker.closeConfirmDesc")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={handleConfirmClose} className="bg-destructive text-destructive-foreground hover:bg-destructive/90">
              {t("common.close")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

// resolve countries via Rust backend (cached per results session)
const countryCache = new Map<string, string>();

async function resolveCountries(phones: string[]): Promise<Map<string, string>> {
  const unknown = phones.filter((p) => p && !countryCache.has(p));
  if (unknown.length > 0) {
    const IS_DEV = !("__TAURI_INTERNALS__" in window);
    if (!IS_DEV) {
      const results = await invoke<string[]>("phones_to_countries", { phones: unknown });
      for (let i = 0; i < unknown.length; i++) {
        countryCache.set(unknown[i], results[i] || "");
      }
    }
  }
  const map = new Map<string, string>();
  for (const p of phones) {
    map.set(p, countryCache.get(p) || "");
  }
  return map;
}

function useCountryMap(accounts: CheckerAccount[]) {
  const [map, setMap] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    const phones = accounts.map((a) => a.phone).filter(Boolean);
    if (phones.length === 0) return;
    resolveCountries(phones).then(setMap);
  }, [accounts]);
  return map;
}

function useCountryGroups(accounts: CheckerAccount[], countryMap: Map<string, string>) {
  return useMemo(() => {
    const groups: Record<string, number> = {};
    for (const acc of accounts) {
      const country = countryMap.get(acc.phone) || "";
      if (country) groups[country] = (groups[country] || 0) + 1;
    }
    return Object.entries(groups).sort((a, b) => b[1] - a[1]);
  }, [accounts, countryMap]);
}

function CountryPieChart({ groups }: { groups: [string, number][] }) {
  if (groups.length === 0) return null;

  let pieData: { value: number; color: string; name: string }[];
  if (groups.length <= 7) {
    pieData = groups.map(([code, count], i) => ({
      value: count,
      color: COUNTRY_COLORS[i % COUNTRY_COLORS.length],
      name: code,
    }));
  } else {
    const top6 = groups.slice(0, 6);
    const othersCount = groups.slice(6).reduce((s, [, c]) => s + c, 0);
    pieData = top6.map(([code, count], i) => ({
      value: count,
      color: COUNTRY_COLORS[i],
      name: code,
    }));
    pieData.push({ value: othersCount, color: COUNTRY_COLORS[6], name: "Other" });
  }

  return (
    <div className="flex flex-col items-center gap-3 shrink-0">
      <PieChart data={pieData} />
      <div className="flex flex-col gap-1 text-sm">
        {pieData.map((item) => (
          <div key={item.name} className="flex items-center gap-2">
            <span className="w-3 h-3 rounded-sm shrink-0" style={{ backgroundColor: item.color }} />
            <span className="text-muted-foreground">{item.name}</span>
            <span className="text-xs text-foreground font-medium">{item.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function CountryBreakdown({ accounts, countryMap }: { accounts: CheckerAccount[]; countryMap: Map<string, string> }) {
  const groups = useCountryGroups(accounts, countryMap);
  if (groups.length <= 1) return null;

  return (
    <div className="flex flex-wrap gap-2 mb-3 pb-2 border-b border-border">
      {groups.map(([code, count]) => (
        <span key={code} className="inline-flex items-center gap-1 rounded-md border border-border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
          <span className="font-medium text-foreground">{code}</span>
          <span>{count}</span>
        </span>
      ))}
    </div>
  );
}

// NFT preview cache (persists while results window is open)
const nftPreviewCache = new Map<string, string | null>();

function useNftPreloader(accounts: CheckerAccount[]) {
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const IS_DEV = !("__TAURI_INTERNALS__" in window);
    if (IS_DEV) return;
    const slugs: string[] = [];
    for (const acc of accounts) {
      for (const link of acc.nft_gifts) {
        const slug = link.split("/nft/")[1] || link;
        if (!nftPreviewCache.has(slug)) slugs.push(slug);
      }
    }
    // preload in batches of 4
    let i = 0;
    const loadNext = () => {
      if (i >= slugs.length) return;
      const batch = slugs.slice(i, i + 4);
      i += 4;
      Promise.all(batch.map((slug) =>
        invoke<string>("fetch_nft_preview", { slug })
          .then((url) => { nftPreviewCache.set(slug, url); })
          .catch(() => { nftPreviewCache.set(slug, null); })
      )).then(() => setTimeout(loadNext, 100));
    };
    loadNext();
  }, [accounts]);
}

function SubMenuPanel({
  type,
  accounts,
  onBack,
  countryMap,
}: {
  type: SubMenuType;
  accounts: CheckerAccount[];
  onBack: () => void;
  countryMap: Map<string, string>;
}) {
  const t = useT();
  const [nftPage, setNftPage] = useState(0);
  const NFT_PER_PAGE = 12;

  const renderContent = () => {
    switch (type) {
      case "channels": {
        const accs = accounts.filter((a) => a.channels.length > 0);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        const allChannels: { ch: { title: string; subscribers: number; id?: string | number; username?: string }; accLabel: string }[] = [];
        for (const acc of accs) {
          for (const ch of acc.channels) {
            allChannels.push({ ch, accLabel: acc.username || String(acc.id) });
          }
        }
        const uniqueChannels = Array.from(
          new Map(allChannels.map((item) => [item.ch.id || item.ch.username || `${item.ch.title}-${item.accLabel}`, item])).values()
        );
        return (
          <div className="flex flex-col gap-1.5">
            {uniqueChannels.map((item, i) => (
              <div key={i} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">{item.ch.title}</span>
                <span className="text-muted-foreground text-xs">({item.ch.subscribers} subs)</span>
                <span className="ml-auto text-xs text-muted-foreground">@{item.accLabel}</span>
              </div>
            ))}
          </div>
        );
      }
      case "groups": {
        const accs = accounts.filter((a) => a.groups.length > 0);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) =>
              acc.groups.map((gr, i) => (
                <div key={`${acc.id}-${i}`} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                  <span className="text-foreground font-medium">{gr.title}</span>
                  <span className="text-muted-foreground text-xs">({gr.members} members)</span>
                  <span className="ml-auto text-xs text-muted-foreground">@{acc.username || acc.id}</span>
                </div>
              ))
            )}
          </div>
        );
      }
      case "nft_gifts": {
        const allGifts: { link: string; acc: CheckerAccount }[] = [];
        for (const acc of accounts) {
          for (const link of acc.nft_gifts) {
            allGifts.push({ link, acc });
          }
        }
        if (allGifts.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        const totalPages = Math.ceil(allGifts.length / NFT_PER_PAGE);
        const pageGifts = allGifts.slice(nftPage * NFT_PER_PAGE, (nftPage + 1) * NFT_PER_PAGE);

        return (
          <div className="flex flex-col gap-4">
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-3">
              {pageGifts.map((g, i) => {
                const slug = g.link.split("/nft/")[1] || g.link;
                return (
                  <NftGiftCard key={`${nftPage}-${i}`} slug={slug} link={g.link} username={g.acc.username || String(g.acc.id)} />
                );
              })}
            </div>
            {totalPages > 1 && (
              <div className="flex items-center justify-center gap-3">
                <button
                  onClick={() => setNftPage((p) => Math.max(0, p - 1))}
                  disabled={nftPage === 0}
                  className="px-3 py-1 text-sm rounded border border-border hover:bg-muted disabled:opacity-30"
                >
                  ←
                </button>
                <span className="text-sm text-muted-foreground">{nftPage + 1} / {totalPages}</span>
                <button
                  onClick={() => setNftPage((p) => Math.min(totalPages - 1, p + 1))}
                  disabled={nftPage >= totalPages - 1}
                  className="px-3 py-1 text-sm rounded border border-border hover:bg-muted disabled:opacity-30"
                >
                  →
                </button>
              </div>
            )}
          </div>
        );
      }
      case "nft_tags": {
        const accs = accounts
          .filter((a) => a.nft_tags.length > 0)
          .sort((a, b) => b.nft_tags.length - a.nft_tags.length);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-primary">{acc.nft_tags.join(", ")}</span>
              </div>
            ))}
          </div>
        );
      }
      case "phone888": {
        const accs = accounts.filter((a) => a.phone888);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-muted-foreground">+{acc.phone}</span>
              </div>
            ))}
          </div>
        );
      }
      case "premium": {
        const accs = accounts.filter((a) => a.premium);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  {acc.premium_until
                    ? `до ${new Date(acc.premium_until * 1000).toLocaleDateString("ru-RU")}`
                    : ""}
                </span>
              </div>
            ))}
          </div>
        );
      }
      case "reg_date": {
        const accs = accounts
          .filter((a) => a.reg_date !== null)
          .sort((a, b) => {
            const ya = extractYear(a.reg_date || "");
            const yb = extractYear(b.reg_date || "");
            return ya - yb;
          });
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="grid grid-cols-2 gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-muted-foreground">{acc.reg_date}</span>
              </div>
            ))}
          </div>
        );
      }
      case "seed": {
        const accs = accounts.filter((a) => a.seed_found);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-muted-foreground font-mono">{acc.seed_text}</span>
              </div>
            ))}
          </div>
        );
      }
      case "pass_files": {
        const entries: { file: string; path: string; acc: CheckerAccount }[] = [];
        for (const acc of accounts) {
          for (let i = 0; i < acc.pass_files.length; i++) {
            entries.push({
              file: acc.pass_files[i],
              path: acc.pass_file_paths[i] || "",
              acc,
            });
          }
        }
        if (entries.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {entries.map((e, i) => (
              <div key={i} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                {e.path ? (
                  <button
                    onClick={() => invoke("open_file_in_editor", { path: e.path })}
                    className="text-primary hover:underline font-medium text-left"
                  >
                    {e.file}
                  </button>
                ) : (
                  <span className="text-foreground font-medium">{e.file}</span>
                )}
                <span className="ml-auto text-xs text-muted-foreground">@{e.acc.username || e.acc.id}</span>
              </div>
            ))}
          </div>
        );
      }
      case "stars": {
        const accs = accounts.filter((a) => a.stars > 0).sort((a, b) => b.stars - a.stars);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-primary font-medium">{acc.stars} stars</span>
              </div>
            ))}
          </div>
        );
      }
      case "crypto_bots": {
        const accs = accounts.filter((a) => a.crypto_bots.send || a.crypto_bots.xrocket);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) => (
              <div key={acc.id} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                <span className="text-foreground font-medium">@{acc.username || "—"}</span>
                <span className="text-xs text-muted-foreground">id={acc.id}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  {[acc.crypto_bots.send && "@send", acc.crypto_bots.xrocket && "@xrocket"].filter(Boolean).join(", ")}
                </span>
              </div>
            ))}
          </div>
        );
      }
      case "channel_balances": {
        const accs = accounts.filter((a) => a.channel_balances && a.channel_balances.length > 0);
        if (accs.length === 0) return <p className="text-sm text-muted-foreground">{t("checker.noResults")}</p>;
        return (
          <div className="flex flex-col gap-1.5">
            {accs.map((acc) =>
              acc.channel_balances.map((bal, i) => (
                <div key={`${acc.id}-${i}`} className="flex items-center gap-2 text-sm py-1 px-2 rounded bg-muted/30">
                  <span className="text-foreground font-medium">{bal.title}</span>
                  <span className="text-xs text-muted-foreground">({bal.type === "channel" ? "ch" : "gr"})</span>
                  <span className="ml-auto flex items-center gap-3 text-xs">
                    {bal.stars > 0 && <span className="text-amber-400 font-medium">{bal.stars} Stars</span>}
                    {bal.ton > 0 && <span className="text-blue-400 font-medium">{bal.ton} TON</span>}
                  </span>
                  <span className="text-xs text-muted-foreground">@{acc.username || acc.id}</span>
                </div>
              ))
            )}
          </div>
        );
      }
      default:
        return null;
    }
  };

  const getSubMenuAccounts = (t: SubMenuType, accs: CheckerAccount[]): CheckerAccount[] => {
    switch (t) {
      case "channels": return accs.filter((a) => a.channels.length > 0);
      case "groups": return accs.filter((a) => a.groups.length > 0);
      case "nft_gifts": return accs.filter((a) => a.nft_gifts.length > 0);
      case "nft_tags": return accs.filter((a) => a.nft_tags.length > 0);
      case "phone888": return accs.filter((a) => a.phone888);
      case "premium": return accs.filter((a) => a.premium);
      case "reg_date": return accs.filter((a) => a.reg_date !== null);
      case "seed": return accs.filter((a) => a.seed_found);
      case "pass_files": return accs.filter((a) => a.pass_files.length > 0);
      case "stars": return accs.filter((a) => a.stars > 0);
      case "crypto_bots": return accs.filter((a) => a.crypto_bots.send || a.crypto_bots.xrocket);
      case "channel_balances": return accs.filter((a) => a.channel_balances && a.channel_balances.length > 0);
      default: return accs;
    }
  };

  const titles: Record<string, string> = {
    channels: t("checker.withChannels"),
    groups: t("checker.withGroups"),
    nft_gifts: t("checker.withNftGifts"),
    nft_tags: t("checker.withNftTags"),
    phone888: t("checker.with888"),
    premium: t("checker.withPremium"),
    reg_date: t("checker.withRegDate"),
    seed: t("checker.withSeed"),
    pass_files: t("checker.withPassFiles"),
    stars: t("checker.withStars"),
    crypto_bots: t("checker.withCryptoBots"),
    channel_balances: t("checker.withChannelBalances"),
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <button
          onClick={onBack}
          className="rounded-md p-1.5 text-muted-foreground hover:text-foreground hover:bg-muted transition"
        >
          <ArrowLeft className="h-4 w-4" />
        </button>
        <h3 className="text-sm font-bold text-foreground">{titles[type || ""] || ""}</h3>
      </div>
      <div className="max-h-[60vh] overflow-y-auto scrollbar-thin">
        <CountryBreakdown accounts={getSubMenuAccounts(type, accounts)} countryMap={countryMap} />
        {renderContent()}
      </div>
    </div>
  );
}

function extractYear(dateStr: string): number {
  const match = dateStr.match(/(\d{4})/);
  return match ? parseInt(match[1]) : 9999;
}

function NftGiftCard({ slug, link, username }: { slug: string; link: string; username: string }) {
  const [imgSrc, setImgSrc] = useState<string | null>(nftPreviewCache.get(slug) ?? null);
  const [loading, setLoading] = useState(!nftPreviewCache.has(slug));

  useEffect(() => {
    if (nftPreviewCache.has(slug)) {
      setImgSrc(nftPreviewCache.get(slug) ?? null);
      setLoading(false);
      return;
    }
    invoke<string>("fetch_nft_preview", { slug })
      .then((dataUrl) => { nftPreviewCache.set(slug, dataUrl); setImgSrc(dataUrl); setLoading(false); })
      .catch(() => { nftPreviewCache.set(slug, null); setLoading(false); });
  }, [slug]);

  const openLink = (e: React.MouseEvent) => {
    e.preventDefault();
    invoke("open_file_in_editor", { path: link });
  };

  return (
    <div className="flex flex-col items-center gap-1.5 p-2 rounded-lg border border-border bg-muted/20">
      {loading ? (
        <div className="w-full aspect-square rounded-md bg-muted animate-pulse" />
      ) : imgSrc ? (
        <img src={imgSrc} alt={slug} className="w-full aspect-square object-cover rounded-md bg-muted" />
      ) : (
        <div className="w-full aspect-square rounded-md bg-muted flex items-center justify-center text-muted-foreground text-xs">
          No preview
        </div>
      )}
      <button
        onClick={openLink}
        className="text-xs text-primary hover:underline truncate max-w-full"
      >
        {slug}
      </button>
      <span className="text-[10px] text-muted-foreground">@{username}</span>
    </div>
  );
}
