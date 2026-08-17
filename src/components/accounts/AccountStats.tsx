import type { LucideIcon } from "lucide-react";

export function StatBlock({ icon: Icon, label, value, active }: {
  icon: LucideIcon; label: string; value: number; active?: boolean;
}) {
  return (
    <div className={`rounded-xl border p-5 transition ${
      active ? "border-primary/40 bg-card" : "border-border bg-card hover:border-primary/30"
    }`}>
      <div className="flex items-center justify-between">
        <span className="text-xs text-muted-foreground">{label}</span>
        <Icon className="h-4 w-4 text-primary" />
      </div>
      <div className="mt-2 text-3xl font-bold">{value.toLocaleString()}</div>
    </div>
  );
}
