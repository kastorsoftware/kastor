import { useState, type MouseEvent, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "@/i18n";
import billingScreenshot from "@/assets/antisafety/billing.jpg";
import apiKeyScreenshot from "@/assets/antisafety/api-key.jpg";
import aidScreenshot from "@/assets/antisafety/telegram-aid.jpg";

export function AntiSafetyGuide({ onClose }: { onClose: () => void }) {
  const t = useT();
  return <>
    <div tabIndex={0} className="max-h-[65vh] space-y-6 overflow-y-auto pr-3 leading-6">
      <p>{t("registrar.antiSafetyGuide.intro")} <GuideLink href="https://antisafety.net">antisafety.net</GuideLink></p>
      <section className="space-y-2">
        <h3 className="font-semibold text-foreground">{t("registrar.antiSafetyGuide.registrationTitle")}</h3>
        <p>{t("registrar.antiSafetyGuide.registration")} <GuideLink href="https://t.me/ex625">@ex625</GuideLink>.</p>
        <p>{t("registrar.antiSafetyGuide.registrationLink")} <GuideLink href="https://github.com/kastorsoftware/kastor">github.com/kastorsoftware/kastor</GuideLink></p>
      </section>
      <section className="space-y-3">
        <h3 className="font-semibold text-foreground">{t("registrar.antiSafetyGuide.billingTitle")}</h3>
        <p>{t("registrar.antiSafetyGuide.billing")}</p>
        <GuideScreenshot src={billingScreenshot} caption={t("registrar.antiSafetyGuide.billingCaption")} />
        <p>{t("registrar.antiSafetyGuide.paymentWarning")}</p>
      </section>
      <section className="space-y-3">
        <h3 className="font-semibold text-foreground">{t("registrar.antiSafetyGuide.keyTitle")}</h3>
        <p>{t("registrar.antiSafetyGuide.key")}</p>
        <GuideScreenshot src={apiKeyScreenshot} caption={t("registrar.antiSafetyGuide.keyCaption")} />
      </section>
      <section className="space-y-3">
        <h3 className="font-semibold text-foreground">{t("registrar.antiSafetyGuide.aidTitle")}</h3>
        <p>{t("registrar.antiSafetyGuide.aid")}</p>
        <GuideScreenshot src={aidScreenshot} caption={t("registrar.antiSafetyGuide.aidCaption")} />
      </section>
      <section className="space-y-2">
        <h3 className="font-semibold text-foreground">{t("registrar.antiSafetyGuide.kastorTitle")}</h3>
        <p>{t("registrar.antiSafetyGuide.kastor")}</p>
        <p>{t("registrar.antiSafetyGuide.email")}</p>
      </section>
      <p className="rounded-lg border border-border bg-background/50 p-3">{t("registrar.antiSafetyGuide.security")}</p>
    </div>
    <div className="mt-5 flex justify-end"><button type="button" onClick={onClose} className="rounded-md border border-border px-4 py-2 text-sm font-medium hover:bg-muted">{t("common.close")}</button></div>
  </>;
}

function GuideScreenshot({ src, caption }: { src: string; caption: string }) {
  return <figure className="space-y-2"><img src={src} alt={caption} loading="lazy" className="h-auto w-full rounded-lg border border-border" /><figcaption className="text-xs leading-5">{caption}</figcaption></figure>;
}

function GuideLink({ href, children }: { href: string; children: ReactNode }) {
  const t = useT();
  const [failed, setFailed] = useState(false);
  function open(event: MouseEvent<HTMLAnchorElement>) {
    if (!("__TAURI_INTERNALS__" in window)) return;
    event.preventDefault();
    setFailed(false);
    void invoke("open_url", { url: href }).catch(() => setFailed(true));
  }
  return <><a href={href} onClick={open} target="_blank" rel="noopener noreferrer" className="break-words text-primary underline underline-offset-2">{children}</a>{failed && <span role="alert" className="ml-2 text-destructive">{t("registrar.antiSafetyGuide.linkFailed")}</span>}</>;
}
