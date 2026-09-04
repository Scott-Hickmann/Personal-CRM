"use client";

import { useEffect, useMemo, useState } from "react";
import { useTheme } from "next-themes";
import { useRouter } from "next/navigation";
import { SigmaContainer, useRegisterEvents, useSetSettings, useSigma } from "@react-sigma/core";
import { useWorkerLayoutForceAtlas2 } from "@react-sigma/layout-forceatlas2";
import { MultiDirectedGraph } from "graphology";
import type Sigma from "sigma";
import { drawDiscNodeLabel, EdgeArrowProgram, NodeCircleProgram } from "sigma/rendering";
import type { Settings } from "sigma/settings";
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

type NodeAttributes = {
  color: string;
  label: string;
  personId: string;
  size: number;
  x: number;
  y: number;
};

type EdgeAttributes = {
  color: string;
  confidence: number;
  label: string;
  size: number;
  type: "arrow";
};

export function NetworkGraph({ overview, focusedPerson }: { overview: Overview; focusedPerson?: string }) {
  const focused = overview.people.find((person) => person.id === focusedPerson);
  const [query, setQuery] = useState(focused?.display_name ?? "");
  const [confidence, setConfidence] = useState([0]);
  const [relationship, setRelationship] = useState("all");
  const [tier, setTier] = useState("all");
  const [activity, setActivity] = useState("all");
  const [layout, setLayout] = useState("organic");
  const [fitRequest, setFitRequest] = useState(0);
  const { resolvedTheme } = useTheme();

  const people = useMemo(() => new Map(overview.people.map((person) => [person.id, person])), [overview.people]);
  const graphPeople = useMemo(() => new Map(overview.graph.nodes.map((node) => [node.id, people.get(node.person_id)])), [overview.graph.nodes, people]);
  const graph = useMemo(() => createGraph(overview, people), [overview, people]);
  const relationshipTypes = useMemo(() => [...new Set(overview.graph.edges.map((edge) => edge.relationship_type))].sort(), [overview.graph.edges]);

  const visibility = useMemo(() => {
    const term = query.trim().toLocaleLowerCase();
    const edgeIds = new Set<string>();
    const nodeIds = new Set<string>();
    overview.graph.edges.forEach((edge, index) => {
      const source = graphPeople.get(edge.source);
      const target = graphPeople.get(edge.target);
      if (!source || !target || source.is_self || target.is_self) return;
      if (edge.confidence < confidence[0] || (relationship !== "all" && edge.relationship_type !== relationship)) return;
      if (tier !== "all" && source.affinity_tier !== tier && target.affinity_tier !== tier) return;
      if (activity !== "all" && source.activity_state !== activity && target.activity_state !== activity) return;
      if (term && ![source, target].some((person) => personSearchText(person).includes(term))) return;
      edgeIds.add(`edge-${index}`);
      nodeIds.add(edge.source);
      nodeIds.add(edge.target);
    });
    return { edgeIds, nodeIds };
  }, [activity, confidence, graphPeople, overview.graph.edges, query, relationship, tier]);

  const focusedNode = useMemo(() => {
    const term = query.trim().toLocaleLowerCase();
    if (!term) return null;
    return overview.graph.nodes.find((node) => graphPeople.get(node.id)?.display_name.toLocaleLowerCase() === term)?.id
      ?? overview.graph.nodes.find((node) => graphPeople.get(node.id) && personSearchText(graphPeople.get(node.id)!).includes(term))?.id
      ?? null;
  }, [graphPeople, overview.graph.nodes, query]);

  const settings = useMemo<Partial<Settings<NodeAttributes, EdgeAttributes>>>(() => ({
    allowInvalidContainer: true,
    defaultEdgeType: "arrow",
    hideEdgesOnMove: true,
    hideLabelsOnMove: true,
    labelFont: "Geist, sans-serif",
    labelRenderedSizeThreshold: 7,
    maxCameraRatio: 3,
    minCameraRatio: 0.08,
    nodeProgramClasses: { circle: NodeCircleProgram },
    edgeProgramClasses: { arrow: EdgeArrowProgram },
    renderEdgeLabels: true,
    stagePadding: 40,
  }), []);

  return <div className="space-y-4 px-5 py-6 sm:px-8 lg:px-10">
    <Card><CardContent className="grid gap-4 p-4 lg:grid-cols-[minmax(14rem,1fr)_10rem_10rem_10rem_12rem_auto] lg:items-end">
      <div className="space-y-2"><Label htmlFor="network-search">Find a person</Label><div className="relative"><Search className="text-muted-foreground absolute top-1/2 left-3 size-4 -translate-y-1/2" /><Input id="network-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Name, identity, or tag…" className="pl-9" /></div></div>
      <GraphSelect label="Relationship" value={relationship} onChange={setRelationship} options={relationshipTypes} />
      <GraphSelect label="Tier" value={tier} onChange={setTier} options={["core", "close", "familiar", "acquaintance", "peripheral"]} />
      <GraphSelect label="Activity" value={activity} onChange={setActivity} options={["active", "cooling", "dormant", "never"]} />
      <div className="space-y-3"><Label>Confidence ≥ {Math.round(confidence[0] * 100)}%</Label><Slider value={confidence} onValueChange={setConfidence} min={0} max={1} step={0.05} /></div>
      <Button variant="outline" onClick={() => setFitRequest((request) => request + 1)}><Focus />Fit</Button>
    </CardContent></Card>

    <div className="flex flex-wrap items-center justify-between gap-3 text-xs"><p className="text-muted-foreground">Showing {visibility.nodeIds.size} people and {visibility.edgeIds.size} relationships</p><div className="flex flex-wrap items-center gap-3">{Object.entries(tierColors).filter(([name]) => name !== "unknown").map(([name, color]) => <span key={name} className="flex items-center gap-1.5"><span className="size-2.5 rounded-full" style={{ backgroundColor: color }} />{titleCase(name)}</span>)}<Select value={layout} onValueChange={setLayout}><SelectTrigger size="sm" className="w-28"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="organic">Organic</SelectItem><SelectItem value="circle">Circle</SelectItem><SelectItem value="grid">Grid</SelectItem></SelectContent></Select></div></div>
    <Card className="relative"><CardContent className="h-[calc(100vh-20rem)] min-h-[34rem] p-0">
      <SigmaContainer<NodeAttributes, EdgeAttributes> graph={graph} settings={settings} className="size-full">
        <GraphController dark={resolvedTheme === "dark"} edgeIds={visibility.edgeIds} fitRequest={fitRequest} focusedNode={focusedNode} layout={layout} nodeIds={visibility.nodeIds} />
      </SigmaContainer>
      {visibility.edgeIds.size === 0 && <div className="bg-background/80 absolute inset-0 grid place-items-center"><div className="text-center"><p className="font-medium">No relationships match</p><p className="text-muted-foreground mt-1 text-sm">Lower the confidence or remove a filter.</p></div></div>}
    </CardContent></Card>
  </div>;
}

function GraphController({ dark, edgeIds, fitRequest, focusedNode, layout, nodeIds }: {
  dark: boolean;
  edgeIds: Set<string>;
  fitRequest: number;
  focusedNode: string | null;
  layout: string;
  nodeIds: Set<string>;
}) {
  const [hoveredNode, setHoveredNode] = useState<string | null>(null);
  const router = useRouter();
  const sigma = useSigma<NodeAttributes, EdgeAttributes>();
  const setSettings = useSetSettings<NodeAttributes, EdgeAttributes>();
  const registerEvents = useRegisterEvents<NodeAttributes, EdgeAttributes>();
  const { isRunning, start: startForceLayout, stop: stopForceLayout } = useWorkerLayoutForceAtlas2({
    settings: { barnesHutOptimize: true, gravity: 1, scalingRatio: 8, slowDown: 4 },
  });

  useEffect(() => {
    registerEvents({
      clickNode: ({ node }) => router.push(`/people/${encodeURIComponent(sigma.getGraph().getNodeAttribute(node, "personId"))}`),
      enterNode: ({ node }) => setHoveredNode(node),
      leaveNode: () => setHoveredNode(null),
    });
  }, [registerEvents, router, sigma]);

  useEffect(() => {
    const activeNode = hoveredNode ?? focusedNode;
    const activeNodes = activeNode && sigma.getGraph().hasNode(activeNode)
      ? new Set([activeNode, ...sigma.getGraph().neighbors(activeNode)])
      : null;
    setSettings({
      defaultEdgeColor: dark ? "#525252" : "#a3a3a3",
      defaultDrawNodeHover: (context, data, settings) => {
        context.beginPath();
        context.fillStyle = data.color;
        context.arc(data.x, data.y, data.size + 2, 0, Math.PI * 2);
        context.fill();
        drawDiscNodeLabel(context, data, settings);
      },
      edgeLabelColor: { color: dark ? "#d4d4d4" : "#404040" },
      edgeReducer: (edge, data) => {
        if (!edgeIds.has(edge)) return { ...data, hidden: true };
        const [source, target] = sigma.getGraph().extremities(edge);
        const active = activeNode !== null && (source === activeNode || target === activeNode);
        return {
          ...data,
          color: active ? (dark ? "#d4d4d4" : "#525252") : (dark ? "#404040" : "#d4d4d4"),
          forceLabel: active,
          hidden: false,
          label: active ? data.label : null,
          size: active ? data.size * 1.8 : data.size,
        };
      },
      labelColor: { color: dark ? "#fafafa" : "#171717" },
      nodeReducer: (node, data) => {
        if (!nodeIds.has(node)) return { ...data, hidden: true };
        if (!activeNodes) return { ...data, hidden: false };
        const active = activeNodes.has(node);
        return {
          ...data,
          color: active ? data.color : (dark ? "#404040" : "#d4d4d4"),
          forceLabel: active,
          hidden: false,
          highlighted: node === activeNode,
          label: active ? data.label : null,
          size: node === activeNode ? data.size * 1.3 : data.size,
        };
      },
    });
  }, [dark, edgeIds, focusedNode, hoveredNode, nodeIds, setSettings, sigma]);

  useEffect(() => {
    if (fitRequest === 0) return;
    fitVisibleNodes(sigma, nodeIds);
  }, [fitRequest, nodeIds, sigma]);

  useEffect(() => {
    stopForceLayout();
    if (layout === "organic") {
      startForceLayout();
      const timer = window.setTimeout(() => {
        stopForceLayout();
      }, 1600);
      return () => window.clearTimeout(timer);
    }
    assignStaticLayout(sigma.getGraph(), layout);
    sigma.refresh();
  }, [layout, sigma, startForceLayout, stopForceLayout]);

  useEffect(() => {
    if (!isRunning) fitVisibleNodes(sigma, nodeIds);
  }, [isRunning, nodeIds, sigma]);

  return null;
}

function createGraph(overview: Overview, people: Map<string, Overview["people"][number]>) {
  const graph = new MultiDirectedGraph<NodeAttributes, EdgeAttributes>();
  const allowedNodes = overview.graph.nodes.filter((node) => {
    const person = people.get(node.person_id);
    return person && !person.is_self;
  });
  allowedNodes.forEach((node, index) => {
    const person = people.get(node.person_id)!;
    const angle = index * Math.PI * (3 - Math.sqrt(5));
    const radius = Math.sqrt(index + 1) * 10;
    graph.addNode(node.id, {
      color: tierColors[person.affinity_tier ?? "unknown"] ?? tierColors.unknown,
      label: node.label,
      personId: node.person_id,
      size: 4 + Math.min(8, (person.affinity_score ?? 0) / 10),
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
    });
  });
  overview.graph.edges.forEach((edge, index) => {
    if (!graph.hasNode(edge.source) || !graph.hasNode(edge.target)) return;
    graph.addEdgeWithKey(`edge-${index}`, edge.source, edge.target, {
      color: "#a3a3a3",
      confidence: edge.confidence,
      label: titleCase(edge.relationship_type),
      size: 0.5 + edge.confidence * 2,
      type: "arrow",
    });
  });
  return graph;
}

function assignStaticLayout(graph: MultiDirectedGraph<NodeAttributes, EdgeAttributes>, layout: string) {
  const nodes = graph.nodes();
  const columns = Math.max(1, Math.ceil(Math.sqrt(nodes.length)));
  nodes.forEach((node, index) => {
    const angle = (index / Math.max(1, nodes.length)) * Math.PI * 2;
    graph.setNodeAttribute(node, "x", layout === "circle" ? Math.cos(angle) * 100 : (index % columns) * 25);
    graph.setNodeAttribute(node, "y", layout === "circle" ? Math.sin(angle) * 100 : Math.floor(index / columns) * 25);
  });
}

function fitVisibleNodes(sigma: Sigma<NodeAttributes, EdgeAttributes>, nodeIds: Set<string>) {
  const graph = sigma.getGraph();
  const positions = [...nodeIds]
    .filter((node) => graph.hasNode(node))
    .map((node) => ({ x: graph.getNodeAttribute(node, "x"), y: graph.getNodeAttribute(node, "y") }));
  if (positions.length === 0) {
    sigma.setCustomBBox(null);
    return;
  }
  const xs = positions.map(({ x }) => x);
  const ys = positions.map(({ y }) => y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const padding = Math.max(maxX - minX, maxY - minY, 1) * 0.08;
  sigma.setCustomBBox({ x: [minX - padding, maxX + padding], y: [minY - padding, maxY + padding] });
  sigma.refresh();
  void sigma.getCamera().animatedReset({ duration: 300 });
}

function personSearchText(person: Overview["people"][number]) {
  return [person.display_name, ...person.tags, ...person.identities].join(" ").toLocaleLowerCase();
}

function GraphSelect({ label, value, onChange, options }: { label: string; value: string; onChange: (value: string) => void; options: string[] }) {
  return <div className="space-y-2"><Label>{label}</Label><Select value={value} onValueChange={onChange}><SelectTrigger className="w-full"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All</SelectItem>{options.map((option) => <SelectItem key={option} value={option}>{titleCase(option)}</SelectItem>)}</SelectContent></Select></div>;
}
