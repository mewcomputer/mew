import { User } from "lucide-react";
import { useSessionStore } from "../stores/session";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup,
  DropdownMenuRadioItem, DropdownMenuTrigger,
} from "./ui/dropdown-menu";

export function PersonaPill() {
  const persona = useSessionStore((s) => s.currentPersona);
  const availablePersonas = useSessionStore((s) => s.availablePersonas);
  const selectPersona = useSessionStore((s) => s.selectPersona);
  const label = persona ?? "default";
  const isCustom = Boolean(persona && persona !== "default");
  const options = [{ name: "default", description: "Use the default agent", color: undefined }, ...availablePersonas.filter((item) => item.name !== "default")];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" title="Persona" className="motion-pressable flex h-7 min-w-0 max-w-32 items-center gap-1.5 rounded-md px-1.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-accent-foreground data-[state=open]:bg-accent">
          <User className="h-3.5 w-3.5 shrink-0" />
          <span className={isCustom ? "truncate capitalize text-foreground" : "sr-only"}>{label}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="start" sideOffset={10} aria-label="Persona" className="w-60 max-w-[calc(100vw-2rem)] rounded-xl p-1.5">
        <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">Persona</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={label} onValueChange={selectPersona}>
          {options.map((option) => (
            <DropdownMenuRadioItem key={option.name} value={option.name} title={option.description} className="min-h-9 rounded-lg text-xs">
              <span className="flex min-w-0 flex-1 items-center gap-2">
                {option.color && <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: option.color }} />}
                <span className="truncate capitalize">{option.name}</span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
