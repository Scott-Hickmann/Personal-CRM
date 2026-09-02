import type { ReactNode } from "react";

export function PageHeader({ eyebrow, title, description, actions }: {
  eyebrow: string;
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <header className="flex flex-col gap-5 border-b px-5 py-8 sm:px-8 lg:flex-row lg:items-end lg:justify-between lg:px-10">
      <div className="max-w-3xl">
        <p className="text-muted-foreground mb-2 font-mono text-[0.7rem] font-medium tracking-[0.18em] uppercase">
          {eyebrow}
        </p>
        <h1 className="text-3xl font-semibold tracking-tight sm:text-4xl">{title}</h1>
        <p className="text-muted-foreground mt-2 text-sm leading-6 sm:text-base">{description}</p>
      </div>
      {actions}
    </header>
  );
}
