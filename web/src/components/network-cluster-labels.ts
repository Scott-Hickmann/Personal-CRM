import { useEffect, type RefObject } from "react";
import { Group } from "three";
import SpriteText from "three-spritetext";
import type { ForceGraphMethods } from "react-force-graph-3d";
import type { Cluster } from "@/lib/clusters";
import type { NetworkData, NetworkLink, NetworkNode } from "@/lib/network";

export function useClusterLabels(scene: RefObject<ForceGraphMethods<NetworkNode, NetworkLink> | undefined>,
  data: NetworkData, clusters: Cluster[], visible: Set<string>, enabled: boolean, selected?: string) {
  useEffect(() => {
    if (!enabled || !scene.current) return;
    const layer = new Group();
    const world = scene.current.scene();
    for (const cluster of clusters) {
      if (selected && selected !== cluster.id) continue;
      const members = new Set(cluster.members);
      const nodes = data.nodes.filter((node) => members.has(node.personId) && visible.has(node.id));
      if (!nodes.length) continue;
      const label = new SpriteText(cluster.name, 3, cluster.color);
      label.position.set(nodes.reduce((sum,n) => sum + n.x, 0) / nodes.length,
        nodes.reduce((sum,n) => sum + n.y, 0) / nodes.length + 12,
        nodes.reduce((sum,n) => sum + n.z, 0) / nodes.length);
      label.material.depthTest = false;
      label.renderOrder = 2;
      layer.add(label);
    }
    world.add(layer);
    return () => {
      world.remove(layer);
      for (const child of layer.children) {
        const label = child as SpriteText;
        label.material.map?.dispose(); label.material.dispose();
      }
    };
  }, [scene, data, clusters, visible, enabled, selected]);
}
