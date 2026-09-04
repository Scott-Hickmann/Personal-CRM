"use client";

import { useMemo, useState } from "react";
import { Focus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { titleCase } from "@/lib/format";
import type { Overview } from "@/lib/types";
import { createNetwork, filterNetwork, tierColors } from "@/lib/network";
import { NetworkScene } from "@/components/network-scene";

export function NetworkGraph({ overview, focusedPerson }: { overview: Overview; focusedPerson?: string }) {
  const [query, setQuery] = useState(overview.people.find((person) => person.id === focusedPerson)?.display_name ?? "");
  const [tier, setTier] = useState("all");
  const [activity, setActivity] = useState("all");
  const [layout, setLayout] = useState("organic");
  const [fitRequest, setFitRequest] = useState(0);
  const graph = useMemo(() => createNetwork(overview), [overview]);
  const visible = useMemo(() => filterNetwork(graph, query, tier, activity), [graph, query, tier, activity]);
  const term = query.trim().toLocaleLowerCase();
  const focused = term ? graph.nodes.find((node) => visible.nodes.has(node.id) && node.label.toLocaleLowerCase() === term)
    ?? graph.nodes.find((node) => visible.nodes.has(node.id) && node.search.includes(term)) : undefined;

  return <div className="space-y-4 px-5 py-6 sm:px-8 lg:px-10">
    <Card><CardContent className="grid gap-4 p-4 lg:grid-cols-[minmax(14rem,1fr)_10rem_10rem_auto] lg:items-end">
      <div className="space-y-2"><Label htmlFor="network-search">Find a person</Label><Input id="network-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Name, identity, or tag…" /></div>
      <GraphSelect label="Tier" value={tier} onChange={setTier} options={["core", "close", "familiar", "acquaintance", "peripheral"]} />
      <GraphSelect label="Activity" value={activity} onChange={setActivity} options={["active", "cooling", "dormant", "never"]} />
      <Button variant="outline" onClick={() => setFitRequest((value) => value + 1)}><Focus />Fit</Button>
    </CardContent></Card>
    <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
      <p className="text-muted-foreground" aria-live="polite">Showing {visible.nodes.size} people and {visible.links.size} relationships</p>
      <div className="flex flex-wrap items-center gap-3">{Object.entries(tierColors).filter(([name]) => name !== "unknown").map(([name, color]) => <span key={name} className="flex items-center gap-1.5"><span className="size-2.5 rounded-full" style={{ backgroundColor: color }} />{titleCase(name)}</span>)}
        <Select value={layout} onValueChange={setLayout}><SelectTrigger aria-label="Layout" size="sm" className="w-28"><SelectValue /></SelectTrigger><SelectContent>{["organic", "sphere", "grid"].map((value) => <SelectItem key={value} value={value}>{titleCase(value)}</SelectItem>)}</SelectContent></Select>
      </div>
    </div>
    <Card className="overflow-hidden"><CardContent className="relative h-[calc(100vh-20rem)] min-h-[34rem] p-0">
      <NetworkScene graph={graph} visible={visible} focused={focused?.id} layout={layout} fitRequest={fitRequest} />
      {visible.links.size === 0 && <div className="bg-background/80 absolute inset-0 grid place-items-center"><div className="text-center"><p className="font-medium">No relationships match</p><p className="text-muted-foreground mt-1 text-sm">Remove a filter or search for another person.</p></div></div>}
    </CardContent></Card>
    <p className="text-muted-foreground text-xs">Drag to orbit · Scroll or pinch to zoom · Right-drag to pan · Click a person to highlight relationships · Click empty space to clear</p>
  </div>;
}

function GraphSelect({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: string[] }) {
  return <div className="space-y-2"><Label>{label}</Label><Select value={value} onValueChange={onChange}><SelectTrigger aria-label={label} className="w-full"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All</SelectItem>{options.map((option) => <SelectItem key={option} value={option}>{titleCase(option)}</SelectItem>)}</SelectContent></Select></div>;
}
