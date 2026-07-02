import { useState, useCallback } from "react";

type Tab = "bash" | "brew";

const commands: Record<Tab, string> = {
    bash: "curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh",
    brew: "brew tap mewcomputer/mew && brew install mew",
};

function ClipboardIcon({ className }: { className?: string }) {
    return (
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            className={className}
        >
            <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
            <path d="M4 16c-1.1 0-2-.9-2-2V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2" />
        </svg>
    );
}

function CheckIcon({ className }: { className?: string }) {
    return (
        <svg
            xmlns="http://www.w3.org/2000/svg"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            className={className}
        >
            <path d="M20 6 9 17l-5-5" />
        </svg>
    );
}

export default function InstallWidget() {
    const [tab, setTab] = useState<Tab>("bash");
    const [copied, setCopied] = useState(false);

    const copy = useCallback(() => {
        navigator.clipboard
            ?.writeText(commands[tab])
            .then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1200);
            })
            .catch(() => {});
    }, [tab]);

    const tabButton = (name: Tab, label: string) => {
        const active = tab === name;
        return (
            <button
                type="button"
                onClick={() => setTab(name)}
                className={[
                    "px-4 py-2 text-sm font-mono border-b-2 transition-colors",
                    active
                        ? "text-foreground border-primary bg-primary/5"
                        : "text-muted-foreground border-transparent hover:text-foreground",
                ].join(" ")}
            >
                {label}
            </button>
        );
    };

    return (
        <div className="bg-card border border-border rounded-lg overflow-hidden shadow-sm">
            <div className="flex border-b border-border">
                {tabButton("bash", "bash")}
                {tabButton("brew", "brew")}
            </div>
            <div className="p-4 flex items-center justify-between gap-4">
                <code className="font-mono text-sm text-foreground break-all">
                    {commands[tab]}
                </code>
                <button
                    type="button"
                    onClick={copy}
                    className="text-muted-foreground hover:text-foreground transition-colors shrink-0"
                    aria-label="Copy command"
                >
                    {copied ? (
                        <CheckIcon />
                    ) : (
                        <ClipboardIcon />
                    )}
                </button>
            </div>
        </div>
    );
}
