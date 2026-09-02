"use client";

import type { ReactNode } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useTheme } from "next-themes";
import { GitFork, Moon, Sun, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const navigation = [
  { href: "/", label: "People", icon: UsersRound },
  { href: "/network", label: "Network", icon: GitFork },
];

export function AppShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const { resolvedTheme, setTheme } = useTheme();

  return (
    <div className="min-h-screen md:grid md:grid-cols-[15rem_1fr]">
      <aside className="border-sidebar-border bg-sidebar/90 supports-[backdrop-filter]:bg-sidebar/80 sticky top-0 z-30 border-b backdrop-blur md:h-screen md:border-r md:border-b-0">
        <div className="flex h-16 items-center justify-between px-4 md:h-auto md:px-5 md:py-6">
          <Link href="/" className="flex items-center gap-3">
            <span className="bg-primary text-primary-foreground grid size-9 place-items-center rounded-lg font-mono text-xs font-semibold">
              CRM
            </span>
            <span>
              <span className="block text-sm font-semibold tracking-tight">Personal CRM</span>
              <span className="text-muted-foreground block text-xs">Private & local</span>
            </span>
          </Link>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Toggle color theme"
            onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
          >
            {resolvedTheme === "dark" ? <Sun /> : <Moon />}
          </Button>
        </div>

        <nav className="flex gap-1 overflow-x-auto px-3 pb-3 md:block md:space-y-1 md:px-3">
          {navigation.map((item) => {
            const active = item.href === "/" ? pathname === "/" : pathname.startsWith(item.href);
            return (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground flex min-w-max items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                  active && "bg-sidebar-accent text-sidebar-accent-foreground",
                )}
              >
                <item.icon className="size-4" />
                {item.label}
              </Link>
            );
          })}
        </nav>

        <div className="text-muted-foreground absolute inset-x-5 bottom-5 hidden text-xs leading-relaxed md:block">
          Every read and write is handled by the Rust CLI.
        </div>
      </aside>
      <main className="min-w-0">{children}</main>
    </div>
  );
}
