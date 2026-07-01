import { useMemo, useState } from "react";
import { THEMES, useTheme, type ThemeDef } from "../lib/theme";
import { cn } from "../lib/utils";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Check, Search } from "lucide-react";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const { themeId, setThemeId } = useTheme();
  const [query, setQuery] = useState("");

  // Group themes by category, preserving order from themes.json.
  const categories = useMemo(() => {
    const map = new Map<string, ThemeDef[]>();
    for (const t of THEMES) {
      const list = map.get(t.category) ?? [];
      list.push(t);
      map.set(t.category, list);
    }
    return map;
  }, []);

  const filtered = useMemo(() => {
    const result = new Map<string, ThemeDef[]>();
    const q = query.toLowerCase();
    for (const [cat, list] of categories) {
      const matches =
        q === ""
          ? list
          : list.filter(
              (t) =>
                t.name.toLowerCase().includes(q) ||
                t.id.toLowerCase().includes(q) ||
                t.description.toLowerCase().includes(q),
            );
      if (matches.length > 0) result.set(cat, matches);
    }
    return result;
  }, [categories, query]);

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[85vh] max-w-2xl gap-0 overflow-hidden p-0">
        <DialogHeader className="border-b border-border px-4 py-3 text-left">
          <DialogTitle className="text-sm">Settings</DialogTitle>
          <DialogDescription className="sr-only">
            Manage theme and display preferences.
          </DialogDescription>
        </DialogHeader>

        {/* Search */}
        <div className="relative border-b border-border px-4 py-2">
          <Search className="pointer-events-none absolute left-6 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search themes…"
            className="h-8 pl-7 text-xs"
          />
        </div>

        {/* Theme grid */}
        <div className="max-h-[55vh] overflow-y-auto p-4">
          {filtered.size === 0 && (
            <div className="py-8 text-center text-xs text-muted-foreground">
              No themes match &ldquo;{query}&rdquo;
            </div>
          )}
          {[...filtered.entries()].map(([category, themes]) => (
            <div key={category} className="mb-5 last:mb-0">
              <div className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                {category}
              </div>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                {themes.map((t) => (
                  <ThemeCard
                    key={t.id}
                    theme={t}
                    active={t.id === themeId}
                    onClick={() => setThemeId(t.id)}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border px-4 py-2">
          <span className="text-[10px] text-muted-foreground">
            {THEMES.length} themes available
          </span>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Done
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ThemeCard({
  theme,
  active,
  onClick,
}: {
  theme: ThemeDef;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "group relative flex flex-col gap-1.5 rounded-lg border p-2.5 text-left transition-all",
        active
          ? "border-primary ring-1 ring-primary"
          : "border-border hover:border-foreground/20 hover:bg-accent",
      )}
    >
      {/* Swatches */}
      <div className="flex h-8 gap-1 overflow-hidden rounded-md">
        {theme.preview.map((color, i) => (
          <div
            key={i}
            className="flex-1"
            style={{ backgroundColor: color }}
          />
        ))}
      </div>

      {/* Label */}
      <div className="min-w-0">
        <div className="truncate text-xs font-medium text-foreground">
          {theme.name}
        </div>
        <div className="truncate text-[10px] text-muted-foreground">
          {theme.description}
        </div>
      </div>

      {active && (
        <div className="absolute right-2 top-2 flex h-4 w-4 items-center justify-center rounded-full bg-primary">
          <Check className="h-2.5 w-2.5 text-primary-foreground" />
        </div>
      )}
    </button>
  );
}
