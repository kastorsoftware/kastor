import { useEffect, useRef, useState } from "react";
import { Bold, Italic, Underline as UnderlineIcon, Strikethrough, Code, Link2, EyeOff, Image as ImageIcon, Video as VideoIcon, X } from "lucide-react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";
import { useT } from "@/i18n";

interface Props {
  open: boolean;
  initialValue: string;
  initialImagePath?: string;
  initialVideoPath?: string;
  title?: string;
  withImage?: boolean;
  withVideo?: boolean;
  onClose: () => void;
  onSave: (text: string, imagePath?: string, videoPath?: string) => void;
}

// markdown wrappers (telegram markdownV2 compatible)
type Wrap = { open: string; close: string; tooltipKey: string; icon: React.ReactNode };

const WRAPS: Wrap[] = [
  { open: "**", close: "**", tooltipKey: "messageEditor.tooltipBold", icon: <Bold className="h-4 w-4" /> },
  { open: "__", close: "__", tooltipKey: "messageEditor.tooltipItalic", icon: <Italic className="h-4 w-4" /> },
  { open: "++", close: "++", tooltipKey: "messageEditor.tooltipUnderline", icon: <UnderlineIcon className="h-4 w-4" /> },
  { open: "~~", close: "~~", tooltipKey: "messageEditor.tooltipStrikethrough", icon: <Strikethrough className="h-4 w-4" /> },
  { open: "||", close: "||", tooltipKey: "messageEditor.tooltipSpoiler", icon: <EyeOff className="h-4 w-4" /> },
  { open: "`", close: "`", tooltipKey: "messageEditor.tooltipMono", icon: <Code className="h-4 w-4" /> },
];

const IS_DEV = !("__TAURI_INTERNALS__" in window);

export function MessageEditorModal({ open, initialValue, initialImagePath, initialVideoPath, title, withImage, withVideo, onClose, onSave }: Props) {
  const t = useT();
  const [text, setText] = useState(initialValue);
  const [imagePath, setImagePath] = useState<string>(initialImagePath || "");
  const [videoPath, setVideoPath] = useState<string>(initialVideoPath || "");
  const taRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open) {
      setText(initialValue);
      setImagePath(initialImagePath || "");
      setVideoPath(initialVideoPath || "");
    }
  }, [open, initialValue, initialImagePath, initialVideoPath]);

  const wrap = (openMark: string, closeMark: string) => {
    const ta = taRef.current;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const before = text.slice(0, start);
    const selected = text.slice(start, end);
    const after = text.slice(end);
    const next = before + openMark + selected + closeMark + after;
    setText(next);

    requestAnimationFrame(() => {
      ta.focus();
      const newStart = start + openMark.length;
      const newEnd = newStart + selected.length;
      ta.setSelectionRange(newStart, newEnd);
    });
  };

  const insertLink = () => {
    const ta = taRef.current;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const selected = text.slice(start, end) || t("messageEditor.linkDefaultText");
    const url = window.prompt(t("messageEditor.linkPrompt"), "https://");
    if (!url) return;
    const before = text.slice(0, start);
    const after = text.slice(end);
    const linkMd = `[${selected}](${url})`;
    setText(before + linkMd + after);
    requestAnimationFrame(() => {
      ta.focus();
      const pos = before.length + linkMd.length;
      ta.setSelectionRange(pos, pos);
    });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "b") { e.preventDefault(); wrap("**", "**"); }
    else if (k === "i") { e.preventDefault(); wrap("__", "__"); }
    else if (k === "u") { e.preventDefault(); wrap("++", "++"); }
    else if (k === "`" || e.key === "`") { e.preventDefault(); wrap("`", "`"); }
  };

  const selectImage = async () => {
    if (IS_DEV) return;
    const { open: openDlg } = await import("@tauri-apps/plugin-dialog");
    const path = await openDlg({ multiple: false, filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp", "gif"] }] });
    if (path) setImagePath(path as string);
  };

  const selectVideo = async () => {
    if (IS_DEV) return;
    const { open: openDlg } = await import("@tauri-apps/plugin-dialog");
    const path = await openDlg({ multiple: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "avi", "mkv", "webm"] }] });
    if (path) setVideoPath(path as string);
  };

  const handleSave = () => {
    onSave(
      text,
      withImage ? (imagePath || undefined) : undefined,
      withVideo ? (videoPath || undefined) : undefined,
    );
    onClose();
  };

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{title || t("messageEditor.defaultTitle")}</DialogTitle>
          <DialogDescription>{t("messageEditor.markdownHint")}</DialogDescription>
        </DialogHeader>

        {/* toolbar */}
        <div className="flex items-center gap-1 border-b border-border pb-2">
          {WRAPS.map((w) => (
            <ToolbarButton key={w.open + w.close} title={t(w.tooltipKey)} onClick={() => wrap(w.open, w.close)}>
              {w.icon}
            </ToolbarButton>
          ))}
          <ToolbarButton title={t("messageEditor.tooltipLink")} onClick={insertLink}>
            <Link2 className="h-4 w-4" />
          </ToolbarButton>
          {withImage && (
            <>
              <div className="ml-1 h-5 w-px bg-border" />
              <ToolbarButton title={t("messageEditor.tooltipImage")} onClick={selectImage}>
                <ImageIcon className="h-4 w-4" />
              </ToolbarButton>
            </>
          )}
          {withVideo && (
            <ToolbarButton title={t("messageEditor.tooltipVideo")} onClick={selectVideo}>
              <VideoIcon className="h-4 w-4" />
            </ToolbarButton>
          )}
        </div>

        {/* editor */}
        <textarea
          ref={taRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          rows={9}
          className="w-full resize-y rounded-md border border-border bg-background px-3 py-2 text-sm font-mono outline-none focus:border-primary/50"
          placeholder={t("messageEditor.placeholder")}
        />

        {/* attached image */}
        {withImage && imagePath && (
          <div className="flex items-center gap-2 rounded-md border border-border bg-card/50 px-3 py-2 text-xs">
            <ImageIcon className="h-4 w-4 text-muted-foreground shrink-0" />
            <span className="truncate text-foreground flex-1">{imagePath.split(/[/\\]/).pop()}</span>
            <button
              onClick={() => setImagePath("")}
              className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground transition"
              title={t("messageEditor.removeImage")}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* attached video */}
        {withVideo && videoPath && (
          <div className="flex items-center gap-2 rounded-md border border-border bg-card/50 px-3 py-2 text-xs">
            <VideoIcon className="h-4 w-4 text-muted-foreground shrink-0" />
            <span className="truncate text-foreground flex-1">{videoPath.split(/[/\\]/).pop()}</span>
            <button
              onClick={() => setVideoPath("")}
              className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground transition"
              title={t("messageEditor.removeVideo")}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* preview */}
        <div className="rounded-md border border-border bg-card/50 p-3 text-sm">
          <div className="mb-1 text-xs font-medium text-muted-foreground">{t("messageEditor.preview")}</div>
          {withImage && imagePath && (
            <div className="mb-2 rounded-md bg-foreground/5 border border-border px-3 py-2 text-xs text-muted-foreground inline-flex items-center gap-1.5">
              <ImageIcon className="h-3.5 w-3.5" />
              {t("messageEditor.imageLabel")}
            </div>
          )}
          {withVideo && videoPath && (
            <div className="mb-2 rounded-md bg-foreground/5 border border-border px-3 py-2 text-xs text-muted-foreground inline-flex items-center gap-1.5">
              <VideoIcon className="h-3.5 w-3.5" />
              {t("messageEditor.videoLabel")}
            </div>
          )}
          <Preview text={text} />
        </div>

        <DialogFooter>
          <button
            onClick={onClose}
            className="rounded-md border border-border px-4 py-1.5 text-sm hover:bg-accent transition"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={handleSave}
            className="rounded-md bg-primary text-primary-foreground px-4 py-1.5 text-sm font-medium hover:bg-primary/90 transition"
          >
            {t("common.save")}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ToolbarButton({ children, title, onClick }: { children: React.ReactNode; title: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      className="rounded-md border border-transparent p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground transition"
    >
      {children}
    </button>
  );
}

function Preview({ text }: { text: string }) {
  const t = useT();
  if (!text.trim()) {
    return <div className="text-muted-foreground italic">{t("messageEditor.empty")}</div>;
  }
  return <div className="whitespace-pre-wrap break-words">{renderMarkdown(text)}</div>;
}

function renderMarkdown(input: string): React.ReactNode[] {
  const tokens: React.ReactNode[] = [];
  let i = 0;
  let key = 0;
  const buf: string[] = [];

  const flushBuf = () => {
    if (buf.length) { tokens.push(buf.join("")); buf.length = 0; }
  };

  while (i < input.length) {
    if (input[i] === "[") {
      const close = input.indexOf("]", i + 1);
      if (close > 0 && input[close + 1] === "(") {
        const urlEnd = input.indexOf(")", close + 2);
        if (urlEnd > 0) {
          flushBuf();
          const linkText = input.slice(i + 1, close);
          const url = input.slice(close + 2, urlEnd);
          tokens.push(<a key={key++} href={url} target="_blank" rel="noreferrer" className="text-primary underline">{renderMarkdown(linkText)}</a>);
          i = urlEnd + 1;
          continue;
        }
      }
    }

    const matched = matchMarker(input, i);
    if (matched) {
      const { mark, end } = matched;
      flushBuf();
      const inner = input.slice(i + mark.length, end);
      tokens.push(wrapNode(mark, renderMarkdown(inner), key++));
      i = end + mark.length;
      continue;
    }

    buf.push(input[i]);
    i++;
  }
  flushBuf();
  return tokens;
}

function matchMarker(input: string, pos: number): { mark: string; end: number } | null {
  const two = input.slice(pos, pos + 2);
  const candidates2 = ["**", "__", "++", "~~", "||"];
  if (candidates2.includes(two)) {
    const end = input.indexOf(two, pos + 2);
    if (end > pos + 1) return { mark: two, end };
  }
  if (input[pos] === "`") {
    const end = input.indexOf("`", pos + 1);
    if (end > pos) return { mark: "`", end };
  }
  return null;
}

function wrapNode(mark: string, children: React.ReactNode, key: number): React.ReactNode {
  switch (mark) {
    case "**": return <b key={key}>{children}</b>;
    case "__": return <i key={key}>{children}</i>;
    case "++": return <u key={key}>{children}</u>;
    case "~~": return <s key={key}>{children}</s>;
    case "||": return <span key={key} className="bg-foreground/15 rounded px-0.5">{children}</span>;
    case "`": return <code key={key} className="rounded bg-foreground/10 px-1 font-mono text-xs">{children}</code>;
    default: return <span key={key}>{children}</span>;
  }
}
