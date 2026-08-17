export function StatusPill({ status }: { status: string }) {
  const colors: Record<string, string> = {
    // Russian
    "Без ограничений": "oklch(0.65 0.1 150)",
    "Заморожен": "oklch(0.65 0.1 220)",
    "Вечный спамблок": "oklch(0.55 0.1 25)",
    "Невалид": "oklch(0.55 0.1 25)",
    "Не проверен": "oklch(0.6 0.05 300)",
    "TData (не конвертирован)": "oklch(0.6 0.1 60)",
    // English
    "No restrictions": "oklch(0.65 0.1 150)",
    "Frozen": "oklch(0.65 0.1 220)",
    "Permanent spamblock": "oklch(0.55 0.1 25)",
    "Invalid": "oklch(0.55 0.1 25)",
    "Unchecked": "oklch(0.6 0.05 300)",
    "TData (not converted)": "oklch(0.6 0.1 60)",
  };
  const isChecking = status.startsWith("Проверка") || status.startsWith("Checking");
  const isReauth = status.startsWith("Переавторизация") || status.startsWith("Re-auth");
  const isTempSpam = status.startsWith("Спамблок по ГЕО") || status.startsWith("Geo spamblock");
  const color = isChecking ? "oklch(0.65 0.1 280)"
    : isReauth ? "oklch(0.65 0.1 280)"
    : isTempSpam ? "oklch(0.7 0.15 85)"
    : (colors[status] || "oklch(0.6 0.05 300)");

  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium ${isChecking || isReauth ? "animate-pulse" : ""}`}
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
