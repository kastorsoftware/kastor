import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

export function AuthKeyImportModal({ onClose, onImported }: { onClose: () => void; onImported: () => void }) {
  const [status, setStatus] = useState<"waiting" | "importing" | "done">("waiting");
  const [filePath, setFilePath] = useState("");
  const [keysFound, setKeysFound] = useState(0);
  const [error, setError] = useState("");
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const mtimeRef = useRef<number | null>(null);

  const IS_DEV = !("__TAURI_INTERNALS__" in window);

  useEffect(() => {
    if (IS_DEV) return;
    invoke<string>("get_authkey_txt_path").then((path) => {
      setFilePath(path);
      invoke("open_file_in_editor", { path });
      invoke<number | null>("get_file_mtime", { path }).then((t) => { mtimeRef.current = t ?? null; });
      intervalRef.current = setInterval(async () => {
        try {
          const newMtime = await invoke<number | null>("get_file_mtime", { path });
          if (newMtime !== mtimeRef.current) {
            mtimeRef.current = newMtime;
            const keys = await invoke<string[]>("read_authkey_txt", { path });
            setKeysFound(keys.length);
          }
        } catch {}
      }, 300);
    });
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, []);

  const handleImport = async () => {
    if (!filePath || keysFound === 0) return;
    setStatus("importing");
    setError("");
    try {
      const keys = await invoke<string[]>("read_authkey_txt", { path: filePath });
      let imported = 0;
      for (const line of keys) {
        const parts = line.split(":");
        const hex = parts[0].trim();
        const dc = parts[1] ? parseInt(parts[1].trim()) : null;
        try {
          await invoke("import_from_authkey", { authKeyHex: hex, dcId: dc && dc >= 1 && dc <= 5 ? dc : null });
          imported++;
        } catch {}
      }
      setStatus("done");
      if (imported > 0) onImported();
      else setError("No keys were imported");
    } catch (e: any) {
      setError(e?.toString() || "Error");
      setStatus("waiting");
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="rounded-xl border border-border bg-card p-6 w-[420px] shadow-2xl relative">
        <button onClick={onClose} className="absolute top-3 right-3 text-muted-foreground hover:text-foreground text-lg leading-none">✕</button>
        <h3 className="text-lg font-semibold mb-3">Auth Key Import</h3>
        <div className="rounded-md border border-border bg-muted/30 px-4 py-3 text-xs text-muted-foreground space-y-1 mb-4">
          <p>File is open in text editor.</p>
          <p>Paste keys (1 line = 1 key), save the file.</p>
          <p>Formats: <code className="text-foreground">authkey_hex</code> or <code className="text-foreground">authkey_hex:dc_id</code></p>
        </div>
        {status === "waiting" && (
          <div className="text-center py-4">
            <div className="text-sm text-muted-foreground mb-2">
              {keysFound > 0
                ? <span>Keys found: <span className="text-foreground font-medium">{keysFound}</span></span>
                : "Waiting for you to add keys..."}
            </div>
            {keysFound > 0 && (
              <button onClick={handleImport} className="rounded-md bg-primary/10 border border-primary/30 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/20 transition">
                Import {keysFound} keys
              </button>
            )}
          </div>
        )}
        {status === "importing" && <div className="text-center py-4 text-sm text-muted-foreground">Importing and detecting DC...</div>}
        {status === "done" && <div className="text-center py-4 text-sm text-[oklch(0.65_0.1_150)]">Import complete</div>}
        {error && <div className="text-center py-2 text-sm text-destructive">{error}</div>}
      </div>
    </div>
  );
}
