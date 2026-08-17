import { useRef, useEffect, useState } from "react";
import { ChevronDown, Search } from "lucide-react";
import { getGeoSearchText, type GeoOption } from "@/lib/countryData";

type FilterOption = GeoOption | { value: string; label: string; separator?: boolean };

export function FilterDropdown({ id, label, value, options, onChange, openFilter, setOpenFilter }: {
  id: string;
  label: string;
  value: string;
  options: FilterOption[];
  onChange: (v: string) => void;
  openFilter: string | null;
  setOpenFilter: (v: string | null) => void;
}) {
  const open = openFilter === id;
  const ref = useRef<HTMLDivElement>(null);
  const [dropSearch, setDropSearch] = useState("");
  const searchable = id === "geo";

  useEffect(() => {
    if (!open) { setDropSearch(""); return; }
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpenFilter(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open, setOpenFilter]);

  const filtered = searchable && dropSearch
    ? options.filter((o) => o.separator || getGeoSearchText(o).includes(dropSearch.toLowerCase()))
    : options;

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpenFilter(open ? null : id)}
        className="flex items-center gap-1.5 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-medium hover:border-primary/50 transition"
      >
        {label}: {value}
        <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
      </button>
      {open && (
        <div className={`absolute top-full left-0 mt-1 z-50 max-h-80 overflow-y-auto scrollbar-thin rounded-md border border-border bg-card shadow-lg py-1 ${searchable ? "min-w-[240px]" : "min-w-[140px]"}`}>
          {searchable && (
            <div className="px-2 pb-1.5 pt-1 sticky top-0 bg-card z-10">
              <div className="relative">
                <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground" />
                <input
                  value={dropSearch}
                  onChange={(e) => setDropSearch(e.target.value)}
                  placeholder="Поиск..."
                  className="w-full rounded border border-border bg-background pl-7 pr-2 py-1 text-xs outline-none focus:border-primary/50"
                  autoFocus
                />
              </div>
            </div>
          )}
          {filtered.map((o, i) => (
            o.separator ? (
              <div key={`sep-${i}`} className="border-t border-border my-1" />
            ) : (
              <button
                key={o.value}
                onClick={() => { onChange(o.value); setOpenFilter(null); }}
                className={`w-full text-left px-3 py-1.5 text-sm hover:bg-accent/50 ${
                  o.label === value ? "text-primary font-medium" : ""
                }`}
              >
                {o.label}
              </button>
            )
          ))}
        </div>
      )}
    </div>
  );
}
