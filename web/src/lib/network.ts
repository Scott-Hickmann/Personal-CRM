import type { Overview } from "./types";

export const tierColors: Record<string, string> = {
  core: "#14b8a6", close: "#3b82f6", familiar: "#8b5cf6",
  acquaintance: "#f59e0b", peripheral: "#737373", unknown: "#a3a3a3",
};
export type NetworkNode = {
  id: string; personId: string; label: string; search: string;
  tier: string; activity: string; color: string; size: number;
  x: number; y: number; z: number; fx?: number; fy?: number; fz?: number;
  vx?: number; vy?: number; vz?: number;
};
export type NetworkLink = {
  id: string; source: string | NetworkNode; target: string | NetworkNode;
  label: string; weight: number;
};
export type NetworkData = { nodes: NetworkNode[]; links: NetworkLink[] };
export const endpointId = (endpoint: string | NetworkNode) => typeof endpoint === "string" ? endpoint : endpoint.id;

export function createNetwork(overview: Overview): NetworkData {
  const people = new Map(overview.people.map((person) => [person.id, person]));
  const nodes = overview.graph.nodes.flatMap((node) => {
    const person = people.get(node.person_id);
    if (!person || person.is_self) return [];
    return [{ id: node.id, personId: node.person_id, label: node.label,
      search: [person.display_name, ...person.tags, ...person.identities].join(" ").toLocaleLowerCase(),
      tier: person.affinity_tier ?? "unknown", activity: person.activity_state ?? "never",
      color: tierColors[person.affinity_tier ?? "unknown"] ?? tierColors.unknown,
      size: 4 + Math.min(8, (person.affinity_score ?? 0) / 10), x: 0, y: 0, z: 0 }];
  });
  const ids = new Set(nodes.map((node) => node.id));
  const degree = new Map<string, number>();
  const links = overview.graph.edges.flatMap((edge, index) => {
    if (!ids.has(edge.source) || !ids.has(edge.target)) return [];
    for (const id of [edge.source, edge.target]) degree.set(id, (degree.get(id) ?? 0) + 1);
    return [{ id: `edge-${index}`, source: edge.source, target: edge.target,
      label: `${edge.shared_context_count} shared ${edge.shared_context_count === 1 ? "conversation" : "conversations"}`,
      weight: Math.max(1, edge.shared_context_count) }];
  });
  nodes.forEach((node) => { node.size += Math.min(5, Math.sqrt(degree.get(node.id) ?? 0)); });
  return { nodes: nodes.filter((node) => degree.has(node.id)), links };
}

export function filterNetwork(graph: NetworkData, tier: string, activity: string) {
  const people = new Map(graph.nodes.map((node) => [node.id, node]));
  const nodes = new Set<string>();
  const links = new Set<string>();
  for (const link of graph.links) {
    const source = people.get(endpointId(link.source))!;
    const target = people.get(endpointId(link.target))!;
    if (tier !== "all" && source.tier !== tier && target.tier !== tier) continue;
    if (activity !== "all" && source.activity !== activity && target.activity !== activity) continue;
    links.add(link.id); nodes.add(source.id); nodes.add(target.id);
  }
  return { nodes, links };
}

export function positionNetwork(graph: NetworkData, layout: string): NetworkData {
  const columns = Math.max(1, Math.ceil(Math.cbrt(graph.nodes.length)));
  const nodes = graph.nodes.map((node, index) => {
    const angle = index * Math.PI * (3 - Math.sqrt(5));
    const height = 1 - 2 * (index + 0.5) / Math.max(1, graph.nodes.length);
    const radius = Math.sqrt(1 - height * height) * 120;
    const x = layout === "grid" ? (index % columns - (columns - 1) / 2) * 35 : Math.cos(angle) * radius;
    const y = layout === "grid" ? (Math.floor(index / columns) % columns - (columns - 1) / 2) * 35 : height * 120;
    const z = layout === "grid" ? (Math.floor(index / columns ** 2) - (columns - 1) / 2) * 35 : Math.sin(angle) * radius;
    return { ...node, x, y, z, fx: layout === "organic" ? undefined : x, fy: layout === "organic" ? undefined : y, fz: layout === "organic" ? undefined : z };
  });
  return { nodes, links: graph.links.map((link) => ({ ...link, source: endpointId(link.source), target: endpointId(link.target) })) };
}

// Keep node sizes legible even when disconnected clusters spread far apart.
export function normalizeNetwork(nodes: NetworkNode[]) {
  if (!nodes.length) return;
  const axes = ["x", "y", "z"] as const;
  const bounds = axes.map((axis) => {
    const values = nodes.map((node) => node[axis]);
    return { min: Math.min(...values), max: Math.max(...values) };
  });
  const scale = 500 / Math.max(1, ...bounds.map(({ min, max }) => max - min));
  for (const node of nodes) axes.forEach((axis, index) => {
    node[axis] = (node[axis] - (bounds[index].min + bounds[index].max) / 2) * scale;
  });
}
