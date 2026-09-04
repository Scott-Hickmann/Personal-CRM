"use client";

import { Component, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import ForceGraph3D, { type ForceGraphMethods } from "react-force-graph-3d";
import SpriteText from "three-spritetext";
import { Group, type PerspectiveCamera } from "three";
import { useTheme } from "next-themes";
import Link from "next/link";
import { endpointId, normalizeNetwork, positionNetwork, type NetworkData, type NetworkNode, type NetworkLink } from "@/lib/network";

type Props = {
  graph: NetworkData; visible: { nodes: Set<string>; links: Set<string> };
  focused?: string; layout: string; fitRequest: number;
};
const motionQuery = "(prefers-reduced-motion: reduce)";
function subscribeMotion(callback: () => void) {
  const query = window.matchMedia(motionQuery);
  query.addEventListener("change", callback);
  return () => query.removeEventListener("change", callback);
}

class SceneBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    return this.state.failed ? <div className="grid h-full place-items-center p-6 text-center" role="alert"><p>The 3D graph could not load. Enable graphics acceleration and reload, or <Link className="underline" href="/">browse people</Link>.</p></div> : this.props.children;
  }
}

export function NetworkScene(props: Props) {
  return <SceneBoundary><Scene key={props.layout} {...props} /></SceneBoundary>;
}

function Scene({ graph, visible, focused, layout, fitRequest }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const scene = useRef<ForceGraphMethods<NetworkNode, NetworkLink>>(undefined);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [selection, setSelection] = useState<{ id?: string; focused?: string }>();
  const [settledData, setSettledData] = useState<NetworkData | null>(null);
  const finalizedData = useRef<NetworkData | null>(null);
  const [contextLost, setContextLost] = useState(false);
  const lastFitRequest = useRef(fitRequest);
  const { resolvedTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  const reducedMotion = useSyncExternalStore(subscribeMotion, () => window.matchMedia(motionQuery).matches, () => false);
  const data = useMemo(() => positionNetwork(graph, layout), [graph, layout]);
  const settled = settledData === data;
  useEffect(() => {
    const instance = scene.current;
    if (instance && layout === "organic") {
      instance.d3Force("link")?.distance((link: NetworkLink) => 65 / Math.sqrt(Math.log2(link.weight + 1)));
      instance.d3Force("charge")?.strength(-100);
      instance.d3Force("gravity", (alpha: number) => {
        for (const node of data.nodes) {
          node.vx = (node.vx ?? 0) - node.x * alpha * 0.025;
          node.vy = (node.vy ?? 0) - node.y * alpha * 0.025;
          node.vz = (node.vz ?? 0) - node.z * alpha * 0.025;
        }
      });
    }
  }, [data, layout, size.width]);
  const selectedId = selection && selection.focused === focused ? selection.id : focused;
  const active = selectedId && visible.nodes.has(selectedId) ? selectedId : undefined;
  const selectedPerson = data.nodes.find((node) => node.id === active);
  const neighbors = useMemo(() => {
    const ids = new Set<string>();
    if (active) {
      ids.add(active);
      for (const link of data.links) {
        if (visible.links.has(link.id) && [endpointId(link.source), endpointId(link.target)].includes(active)) {
          ids.add(endpointId(link.source)); ids.add(endpointId(link.target));
        }
      }
    }
    return ids;
  }, [active, data, visible]);
  const activeLink = useCallback((link: NetworkLink) => Boolean(active && [endpointId(link.source), endpointId(link.target)].includes(active)), [active]);
  const frame = useCallback((focus = true) => {
    if (!scene.current || !visible.nodes.size) return;
    const node = focus ? data.nodes.find((node) => node.id === focused) : undefined;
    const duration = reducedMotion ? 0 : 450;
    if (node) scene.current.cameraPosition({ x: node.x, y: node.y, z: node.z + 180 }, node, duration);
    else {
      const nodes = data.nodes.filter((node) => visible.nodes.has(node.id));
      const bounds = (["x", "y", "z"] as const).map((axis) => {
        const values = nodes.map((node) => node[axis]);
        return { min: Math.min(...values), max: Math.max(...values) };
      });
      const center = { x: (bounds[0].min + bounds[0].max) / 2, y: (bounds[1].min + bounds[1].max) / 2, z: (bounds[2].min + bounds[2].max) / 2 };
      const radius = Math.max(20, ...nodes.map((node) => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z) + node.size));
      const camera = scene.current.camera() as PerspectiveCamera;
      const halfFov = camera.fov * Math.PI / 360;
      const distance = radius * 1.1 / Math.sin(Math.min(halfFov, Math.atan(Math.tan(halfFov) * camera.aspect)));
      scene.current.cameraPosition({ ...center, z: center.z + distance }, center, duration);
    }
  }, [data, focused, reducedMotion, visible]);

  useEffect(() => {
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    if (container.current) observer.observe(container.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const requested = fitRequest !== lastFitRequest.current;
    lastFitRequest.current = fitRequest;
    if (settled) frame(!requested);
  }, [fitRequest, frame, settled]);
  useEffect(() => {
    const canvas = scene.current?.renderer().domElement;
    const lost = () => setContextLost(true);
    canvas?.addEventListener("webglcontextlost", lost);
    return () => canvas?.removeEventListener("webglcontextlost", lost);
  }, [size.width, size.height]);

  const labelObject = useCallback((node: NetworkNode) => {
    if (!neighbors.has(node.id)) return new Group();
    const label = new SpriteText(node.label, 4, dark ? "#fafafa" : "#171717");
    label.position.y = 8;
    label.material.depthTest = false;
    label.renderOrder = 1;
    return label;
  }, [dark, neighbors]);
  const edgeObject = useCallback((link: NetworkLink) => {
    if (!activeLink(link)) return new Group();
    const label = new SpriteText(link.label, 2.5, dark ? "#e5e5e5" : "#404040");
    label.material.depthTest = false;
    label.renderOrder = 1;
    return label;
  }, [activeLink, dark]);

  return <div ref={container} className="size-full" aria-label="3D relationship network">
    {size.width > 0 && <ForceGraph3D<NetworkNode, NetworkLink>
      ref={scene} width={size.width} height={size.height} graphData={data}
      controlType="orbit" backgroundColor={dark ? "#171717" : "#ffffff"}
      showNavInfo={false} enableNodeDrag={false} nodeResolution={8}
      nodeVisibility={(node) => visible.nodes.has(node.id)} linkVisibility={(link) => visible.links.has(link.id)}
      nodeVal={(node) => (node.size / 4) ** 3} nodeRelSize={2}
      nodeColor={(node) => active && !neighbors.has(node.id) ? (dark ? "#404040" : "#d4d4d4") : node.color}
      nodeLabel={() => ""} nodeThreeObject={labelObject} nodeThreeObjectExtend
      linkColor={(link) => activeLink(link) ? (dark ? "#e5e5e5" : "#404040") : (dark ? "#555555" : "#bbbbbb")}
      linkWidth={(link) => activeLink(link) ? 0.5 + Math.min(1.5, Math.log2(link.weight + 1) * 0.2) : 0}
      linkOpacity={0.5} linkThreeObject={edgeObject} linkThreeObjectExtend
      linkPositionUpdate={(object, { start, end }) => { object.position.set((start.x + end.x) / 2, (start.y + end.y) / 2, (start.z + end.z) / 2); }}
      onNodeClick={(node) => setSelection({ id: node.id === active ? undefined : node.id, focused })}
      onBackgroundClick={() => setSelection({ focused })}
      warmupTicks={layout === "organic" ? 100 : 0} cooldownTicks={layout === "organic" && !reducedMotion ? 100 : 0}
      onEngineStop={() => {
        // Selection styling updates also stop the engine again.
        if (finalizedData.current === data) return;
        finalizedData.current = data;
        if (layout === "organic") normalizeNetwork(data.nodes);
        setSettledData(data);
      }}
    />}
    {selectedPerson && <div className="absolute top-4 right-4 flex max-w-[calc(100%-2rem)] flex-wrap items-center gap-3 rounded-lg border bg-background/95 px-3 py-2 text-sm shadow-sm">
      <span className="font-medium" aria-live="polite">{selectedPerson.label}</span>
      <Link className="underline underline-offset-4" href={`/people/${encodeURIComponent(selectedPerson.personId)}`}>Open profile</Link>
      <button className="text-muted-foreground hover:text-foreground" onClick={() => setSelection({ focused })} aria-label="Clear selection">Clear</button>
    </div>}
    {!settled && <div className="pointer-events-none absolute top-4 left-4 rounded bg-background/80 p-2 text-sm" role="status">Arranging network…</div>}
    {contextLost && <div className="bg-background absolute inset-0 grid place-items-center p-6" role="alert">Graphics connection lost. Reload to restore the graph.</div>}
  </div>;
}
