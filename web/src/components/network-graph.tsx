"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useTheme } from "next-themes";
import { useRouter } from "next/navigation";
import cytoscape, { type Core, type EdgeSingular, type LayoutOptions, type NodeSingular } from "cytoscape";
import { Focus, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { titleCase } from "@/lib/format";
import type { Overview } from "@/lib/types";

const tierColors: Record<string, string> = {
  core: "#14b8a6", close: "#3b82f6", familiar: "#8b5cf6",
  acquaintance: "#f59e0b", peripheral: "#737373", unknown: "#a3a3a3",
};

export function NetworkGraph({ overview, focusedPerson }: { overview: Overview; focusedPerson?: string }) {
  const focused = overview.people.find((person) => person.id === focusedPerson);
  const [query, setQuery] = useState(focused?.display_name ?? "");
  const [confidence, setConfidence] = useState([0]);
  const [relationship, setRelationship] = useState("all");
  const [tier, setTier] = useState("all");
  const [activity, setActivity] = useState("all");
  const [layout, setLayout] = useState("cose");
  const container = useRef<HTMLDivElement>(null);
  const instance = useRef<Core | null>(null);
  const { resolvedTheme } = useTheme();
  const router = useRouter();

  const people = useMemo(() => new Map(overview.people.map((person) => [person.id, person])), [overview.people]);
  const graphPeople = useMemo(() => new Map(overview.graph.nodes.map((node) => [node.id, people.get(node.person_id)])), [overview.graph.nodes, people]);
  const relationshipTypes = useMemo(() => [...new Set(overview.graph.edges.map((edge) => edge.relationship_type))].sort(), [overview.graph.edges]);

  const elements = useMemo(() => {
    const term = query.trim().toLocaleLowerCase();
    const isAllowedNode = (nodeId: string) => {
      const person = graphPeople.get(nodeId);
      return Boolean(person && !person.is_self);
    };
    const personMatches = (nodeId: string) => {
      const person = graphPeople.get(nodeId);
      return person && !person.is_self
        && (tier === "all" || person.affinity_tier === tier)
        && (activity === "all" || person.activity_state === activity);
    };
    const edges = overview.graph.edges.filter((edge) => {
      if (edge.confidence < confidence[0] || (relationship !== "all" && edge.relationship_type !== relationship)) return false;
      if (!isAllowedNode(edge.source) || !isAllowedNode(edge.target)) return false;
      if ((tier !== "all" || activity !== "all") && !personMatches(edge.source) && !personMatches(edge.target)) return false;
      if (!term) return true;
      const source = graphPeople.get(edge.source);
      const target = graphPeople.get(edge.target);
      return [source, target].some((person) => person && [person.display_name, ...person.tags, ...person.identities].join(" ").toLocaleLowerCase().includes(term));
    });
    const nodeIds = new Set(edges.flatMap((edge) => [edge.source, edge.target]));
    const nodes = overview.graph.nodes.filter((node) => nodeIds.has(node.id)).map((node) => {
      const person = people.get(node.person_id);
      return { data: { id: node.id, personId: node.person_id, label: node.label, tier: person?.affinity_tier ?? "unknown", score: person?.affinity_score ?? 0 } };
    });
    return [...nodes, ...edges.map((edge, index) => ({ data: { id: `edge-${index}`, source: edge.source, target: edge.target, label: edge.relationship_type, confidence: edge.confidence } }))];
  }, [activity, confidence, graphPeople, overview.graph.edges, overview.graph.nodes, people, query, relationship, tier]);

  useEffect(() => {
    if (!container.current) return;
    const dark = resolvedTheme === "dark";
    instance.current?.destroy();
    instance.current = cytoscape({
      container: container.current,
      elements,
      minZoom: 0.08,
      maxZoom: 3,
      style: [
        { selector: "node", style: { "background-color": (element: NodeSingular) => tierColors[element.data("tier")] ?? tierColors.unknown, label: "data(label)", color: dark ? "#fafafa" : "#171717", "font-family": "Geist, sans-serif", "font-size": 11, "text-valign": "bottom", "text-margin-y": 8, width: (element: NodeSingular) => 22 + Math.min(28, element.data("score") / 3), height: (element: NodeSingular) => 22 + Math.min(28, element.data("score") / 3), "border-width": 2, "border-color": dark ? "#262626" : "#ffffff", "overlay-opacity": 0 } },
        { selector: "node:selected", style: { "border-width": 4, "border-color": dark ? "#fafafa" : "#171717" } },
        { selector: "edge", style: { width: (element: EdgeSingular) => 0.5 + element.data("confidence") * 2.5, "line-color": dark ? "#525252" : "#a3a3a3", "target-arrow-color": dark ? "#525252" : "#a3a3a3", "target-arrow-shape": "triangle", "curve-style": "bezier", opacity: 0.65, label: "data(label)", "font-size": 8, color: dark ? "#a3a3a3" : "#525252", "text-background-color": dark ? "#171717" : "#ffffff", "text-background-opacity": 0.8, "text-background-padding": "2px" } },
      ],
      layout: layoutOptions(layout),
    });
    instance.current.on("tap", "node", (event) => router.push(`/people/${encodeURIComponent(event.target.data("personId"))}`));
    return () => { instance.current?.destroy(); instance.current = null; };
  }, [elements, layout, resolvedTheme, router]);

  const nodeCount = elements.filter((element) => !("source" in element.data)).length;
  const edgeCount = elements.length - nodeCount;

  return <div className="space-y-4 px-5 py-6 sm:px-8 lg:px-10">
    <Card><CardContent className="grid gap-4 p-4 lg:grid-cols-[minmax(14rem,1fr)_10rem_10rem_10rem_12rem_auto] lg:items-end">
      <div className="space-y-2"><Label htmlFor="network-search">Find a person</Label><div className="relative"><Search className="text-muted-foreground absolute top-1/2 left-3 size-4 -translate-y-1/2" /><Input id="network-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Name, identity, or tag…" className="pl-9" /></div></div>
      <GraphSelect label="Relationship" value={relationship} onChange={setRelationship} options={relationshipTypes} />
      <GraphSelect label="Tier" value={tier} onChange={setTier} options={["core", "close", "familiar", "acquaintance", "peripheral"]} />
      <GraphSelect label="Activity" value={activity} onChange={setActivity} options={["active", "cooling", "dormant", "never"]} />
      <div className="space-y-3"><Label>Confidence ≥ {Math.round(confidence[0] * 100)}%</Label><Slider value={confidence} onValueChange={setConfidence} min={0} max={1} step={0.05} /></div>
      <Button variant="outline" onClick={() => instance.current?.fit(undefined, 40)}><Focus />Fit</Button>
    </CardContent></Card>

    <div className="flex flex-wrap items-center justify-between gap-3 text-xs"><p className="text-muted-foreground">Showing {nodeCount} people and {edgeCount} relationships</p><div className="flex flex-wrap items-center gap-3">{Object.entries(tierColors).filter(([name]) => name !== "unknown").map(([name, color]) => <span key={name} className="flex items-center gap-1.5"><span className="size-2.5 rounded-full" style={{ backgroundColor: color }} />{titleCase(name)}</span>)}<Select value={layout} onValueChange={setLayout}><SelectTrigger size="sm" className="w-28"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="cose">Organic</SelectItem><SelectItem value="circle">Circle</SelectItem><SelectItem value="grid">Grid</SelectItem></SelectContent></Select></div></div>
    <Card className="relative"><CardContent className="p-0"><div ref={container} className="h-[calc(100vh-20rem)] min-h-[34rem] w-full" />{elements.length === 0 && <div className="bg-background/80 absolute inset-0 grid place-items-center"><div className="text-center"><p className="font-medium">No relationships match</p><p className="text-muted-foreground mt-1 text-sm">Lower the confidence or remove a filter.</p></div></div>}</CardContent></Card>
  </div>;
}

function GraphSelect({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: string[] }) {
  return <div className="space-y-2"><Label>{label}</Label><Select value={value} onValueChange={onChange}><SelectTrigger className="w-full"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All</SelectItem>{options.map((option) => <SelectItem key={option} value={option}>{titleCase(option)}</SelectItem>)}</SelectContent></Select></div>;
}

function layoutOptions(name: string): LayoutOptions {
  if (name === "circle") return { name: "circle", padding: 40, animate: false };
  if (name === "grid") return { name: "grid", padding: 40, animate: false };
  return { name: "cose", padding: 50, animate: false, nodeRepulsion: () => 9000, idealEdgeLength: () => 110, gravity: 0.25, numIter: 1200 };
}
