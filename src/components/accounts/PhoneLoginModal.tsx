import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "@/i18n";

export function PhoneLoginModal({ onClose, onSuccess }: { onClose: () => void; onSuccess: () => void }) {
  const t = useT();
  const [step, setStep] = useState<"phone" | "code" | "password">("phone");
  const [phone, setPhone] = useState("+");
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [sessionId, setSessionId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  function translateError(raw: string): string {
    const s = raw.replace(/^Error:\s*/, "").trim();
    if (s.includes("PHONE_NUMBER_INVALID")) return t("phoneLogin.phoneInvalid");
    if (s.includes("PHONE_NUMBER_BANNED")) return t("phoneLogin.phoneBanned");
    if (s.includes("PHONE_NUMBER_FLOOD")) return t("phoneLogin.phoneFlood");
    if (s.includes("PHONE_CODE_INVALID")) return t("phoneLogin.codeInvalid");
    if (s.includes("PHONE_CODE_EXPIRED")) return t("phoneLogin.codeExpired");
    if (s.includes("PASSWORD_HASH_INVALID")) return t("phoneLogin.passwordInvalid");
    if (s.startsWith("FLOOD_WAIT:")) return t("phoneLogin.floodWait", { seconds: s.slice("FLOOD_WAIT:".length) });
    if (s.includes("duplicate")) return t("phoneLogin.duplicate");
    if (s.includes("DH key exchange")) return t("phoneLogin.dhError");
    if (s.includes("Не удалось подключиться") || s.includes("Could not connect")) return t("phoneLogin.connectError");
    if (s.includes("Сессия истекла") || s.includes("Session expired")) return t("phoneLogin.sessionExpired");
    return s;
  }

  const sendCode = async () => {
    setError("");
    setBusy(true);
    try {
      const cleaned = phone.replace(/[^\d]/g, "");
      if (cleaned.length < 5) { setError(t("phoneLogin.invalidFormat")); setBusy(false); return; }
      const resp = await invoke<{ session_id: string; code_type: string }>("auth_send_code", { phone: cleaned });
      setSessionId(resp.session_id);
      setStep("code");
    } catch (e: any) {
      setError(translateError(String(e)));
    } finally {
      setBusy(false);
    }
  };

  const signIn = async () => {
    setError("");
    setBusy(true);
    try {
      const resp = await invoke<{ account_id: string | null; two_fa_required: boolean; hint: string }>("auth_sign_in", {
        sessionId, code: code.trim()
      });
      if (resp.two_fa_required) {
        setStep("password");
      } else if (resp.account_id) {
        onSuccess();
      } else {
        setError(t("phoneLogin.loginFailed"));
      }
    } catch (e: any) {
      setError(translateError(String(e)));
    } finally {
      setBusy(false);
    }
  };

  const checkPassword = async () => {
    setError("");
    setBusy(true);
    try {
      await invoke<string>("auth_check_password", { sessionId, password });
      onSuccess();
    } catch (e: any) {
      setError(translateError(String(e)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
         onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="rounded-xl border border-border bg-card p-6 w-[420px] shadow-2xl relative">
        <button onClick={onClose} className="absolute top-3 right-3 text-muted-foreground hover:text-foreground text-lg leading-none">✕</button>

        {step === "phone" && (
          <div className="space-y-3">
            <input
              type="tel"
              value={phone}
              onChange={(e) => setPhone(e.target.value)}
              placeholder="+7..."
              autoFocus
              disabled={busy}
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:border-primary outline-none"
              onKeyDown={(e) => { if (e.key === "Enter") sendCode(); }}
            />
            <button
              onClick={sendCode}
              disabled={busy}
              className="w-full rounded-md bg-primary/10 border border-primary/30 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
            >
              {busy ? t("phoneLogin.connecting") : t("phoneLogin.sendCode")}
            </button>
          </div>
        )}

        {step === "code" && (
          <div className="space-y-3">
            <p className="text-xs text-muted-foreground">{phone}</p>
            <input
              type="text"
              inputMode="numeric"
              maxLength={6}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/[^\d]/g, ""))}
              placeholder="12345"
              autoFocus
              disabled={busy}
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:border-primary outline-none tracking-widest text-center"
              onKeyDown={(e) => { if (e.key === "Enter" && code.length >= 5) signIn(); }}
            />
            <button
              onClick={signIn}
              disabled={busy || code.length < 5}
              className="w-full rounded-md bg-primary/10 border border-primary/30 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
            >
              {busy ? t("phoneLogin.loggingIn") : t("phoneLogin.login")}
            </button>
          </div>
        )}

        {step === "password" && (
          <div className="space-y-3">
            <p className="text-xs text-muted-foreground">2FA</p>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoFocus
              disabled={busy}
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:border-primary outline-none"
              onKeyDown={(e) => { if (e.key === "Enter" && password.length > 0) checkPassword(); }}
            />
            <button
              onClick={checkPassword}
              disabled={busy || password.length === 0}
              className="w-full rounded-md bg-primary/10 border border-primary/30 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/20 transition disabled:opacity-50"
            >
              {busy ? t("phoneLogin.checking") : t("phoneLogin.confirm")}
            </button>
          </div>
        )}

        {error && <div className="mt-3 text-sm text-destructive text-center">{error}</div>}
      </div>
    </div>
  );
}
