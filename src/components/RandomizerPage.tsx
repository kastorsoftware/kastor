import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Check } from "lucide-react";
import { useT } from "@/i18n";

type Mode = "standard" | "aggressive" | "custom";

const IS_DEV = !("__TAURI_INTERNALS__" in window);

export function RandomizerPage() {
  const t = useT();
  const [mode, setMode] = useState<Mode>("standard");
  const [input, setInput] = useState("");
  const [output, setOutput] = useState("");
  const [customPairs, setCustomPairs] = useState("O:О\na:а\ne:е\ns:$");
  const [chance, setChance] = useState(50);
  const [copied, setCopied] = useState(false);

  const handleRandomize = async () => {
    if (!input.trim()) return;
    if (IS_DEV) {
      setOutput(input.split("").reverse().join(""));
      return;
    }
    try {
      const result = await invoke<string>("randomize_text", {
        req: {
          mode,
          text: input,
          custom_pairs: mode === "custom" ? customPairs : null,
          chance,
        },
      });
      setOutput(result);
    } catch (e: any) {
      setOutput(`${t("common.error")}: ${e}`);
    }
  };

  const handleCopy = () => {
    navigator.clipboard.writeText(output);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">{t("descriptions.randomizer")}</p>
      {/* mode tabs */}
      <div className="flex gap-2 flex-wrap">
        <ModeTab active={mode === "standard"} onClick={() => setMode("standard")}>{t("randomizer.modeStandard")}</ModeTab>
        <ModeTab active={mode === "aggressive"} onClick={() => setMode("aggressive")}>{t("randomizer.modeAggressive")}</ModeTab>
        <ModeTab active={mode === "custom"} onClick={() => setMode("custom")}>{t("randomizer.modeCustom")}</ModeTab>
      </div>

      {/* description */}
      <div className="text-xs text-muted-foreground">
        {mode === "standard" && t("randomizer.descStandard")}
        {mode === "aggressive" && t("randomizer.descAggressive")}
        {mode === "custom" && t("randomizer.descCustom")}
      </div>

      <div className="rounded-xl border border-border bg-card overflow-hidden">
        <div className="p-6 space-y-5">

          {/* chance slider */}
          <div>
            <div className="flex items-center gap-4">
              <label className="text-sm font-medium text-foreground whitespace-nowrap shrink-0">{t("randomizer.chance")}: <span className="inline-block w-8 text-right">{chance}</span>%</label>
              <input
                type="range"
                min={1}
                max={100}
                value={chance}
                onChange={(e) => setChance(Number(e.target.value))}
                className="flex-1 min-w-0 accent-primary"
              />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              {t("randomizer.chanceHint")}
            </p>
          </div>

          {/* custom pairs input */}
          {mode === "custom" && (
            <div>
              <label className="text-sm font-medium text-foreground">{t("randomizer.customPairs")}</label>
              <textarea
                value={customPairs}
                onChange={(e) => setCustomPairs(e.target.value)}
                placeholder={"O:О\na:а\ns:$"}
                rows={5}
                className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm font-mono outline-none focus:border-primary/50 resize-y"
              />
              <p className="mt-1 text-xs text-muted-foreground">{t("randomizer.customPairsHint")}</p>
            </div>
          )}

          {/* input text */}
          <div>
            <label className="text-sm font-medium text-foreground">{t("randomizer.inputText")}</label>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder={t("randomizer.inputPlaceholder")}
              rows={6}
              className="mt-1.5 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-primary/50 resize-y"
            />
          </div>

          {/* action */}
          <button
            onClick={handleRandomize}
            disabled={!input.trim()}
            className="rounded-md border border-primary/50 bg-primary/10 px-5 py-2 text-sm text-primary font-medium hover:bg-primary/20 transition disabled:opacity-50"
          >
            {t("randomizer.randomizeBtn")}
          </button>

          {/* output */}
          {output && (
            <div>
              <div className="flex items-center justify-between mb-1.5">
                <label className="text-sm font-medium text-foreground">{t("randomizer.result")}</label>
                <button
                  onClick={handleCopy}
                  className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition"
                >
                  {copied ? <Check className="h-3.5 w-3.5 text-green-500" /> : <Copy className="h-3.5 w-3.5" />}
                  {copied ? t("randomizer.copied") : t("randomizer.copy")}
                </button>
              </div>
              <div className="w-full rounded-md border border-border bg-muted/30 px-3 py-2 text-sm whitespace-pre-wrap break-all select-text min-h-[4rem]">
                {output}
              </div>
            </div>
          )}

        </div>
      </div>
    </div>
  );
}

function ModeTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-md border px-4 py-2 text-sm font-medium transition ${
        active
          ? "border-primary/50 bg-primary/10 text-primary"
          : "border-border bg-background text-muted-foreground hover:border-primary/30"
      }`}
    >
      {children}
    </button>
  );
}
