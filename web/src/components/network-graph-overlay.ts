import { useEffect, useRef } from "react";
import { useSigma } from "@react-sigma/core";
import { drawDiscNodeLabel, drawStraightEdgeLabel } from "sigma/rendering";
import type { EdgeAttributes, NodeAttributes } from "@/components/network-graph";

const layerId = "active-relationships";

export function useActiveGraphOverlay(activeNode: string | null) {
  const sigma = useSigma<NodeAttributes, EdgeAttributes>();
  const activeNodeRef = useRef(activeNode);

  useEffect(() => {
    activeNodeRef.current = activeNode;
    sigma.scheduleRender();
  }, [activeNode, sigma]);

  useEffect(() => {
    const canvas = sigma.createCanvas(layerId, {
      afterLayer: "hoverNodes",
      style: { pointerEvents: "none" },
    });
    const context = canvas.getContext("2d");
    const killLayer = () => {
      if (sigma.getCanvases()[layerId]) sigma.killLayer(layerId);
    };
    if (!context) return killLayer;

    const draw = () => {
      const { width, height } = sigma.getDimensions();
      const pixelRatio = window.devicePixelRatio || 1;
      if (canvas.width !== width * pixelRatio || canvas.height !== height * pixelRatio) {
        canvas.width = width * pixelRatio;
        canvas.height = height * pixelRatio;
        canvas.style.width = `${width}px`;
        canvas.style.height = `${height}px`;
      }
      context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
      context.clearRect(0, 0, width, height);

      const node = activeNodeRef.current;
      const graph = sigma.getGraph();
      if (!node || !graph.hasNode(node)) return;

      const settings = sigma.getSettings();
      const activeEdges = graph.edges(node).filter((edge) => !sigma.getEdgeDisplayData(edge)?.hidden);
      const activeNodes = new Set<string>([node]);

      activeEdges.forEach((edge) => {
        const [source, target] = graph.extremities(edge);
        const sourceData = sigma.getNodeDisplayData(source);
        const targetData = sigma.getNodeDisplayData(target);
        const edgeData = sigma.getEdgeDisplayData(edge);
        if (!sourceData || !targetData || !edgeData || sourceData.hidden || targetData.hidden) return;

        activeNodes.add(source);
        activeNodes.add(target);
        const sourcePosition = sigma.framedGraphToViewport(sourceData);
        const targetPosition = sigma.framedGraphToViewport(targetData);
        const sourceSize = sigma.scaleSize(sourceData.size);
        const targetSize = sigma.scaleSize(targetData.size);
        const dx = targetPosition.x - sourcePosition.x;
        const dy = targetPosition.y - sourcePosition.y;
        const distance = Math.hypot(dx, dy);
        if (distance === 0) return;

        context.beginPath();
        context.moveTo(sourcePosition.x + dx * sourceSize / distance, sourcePosition.y + dy * sourceSize / distance);
        context.lineTo(targetPosition.x - dx * targetSize / distance, targetPosition.y - dy * targetSize / distance);
        context.lineCap = "round";
        context.lineWidth = Math.max(settings.minEdgeThickness, sigma.scaleSize(edgeData.size));
        context.strokeStyle = edgeData.color;
        context.stroke();
      });

      activeEdges.forEach((edge) => {
        const [source, target] = graph.extremities(edge);
        const sourceData = sigma.getNodeDisplayData(source);
        const targetData = sigma.getNodeDisplayData(target);
        const edgeData = sigma.getEdgeDisplayData(edge);
        if (!sourceData || !targetData || !edgeData || !edgeData.label || sourceData.hidden || targetData.hidden) return;
        drawStraightEdgeLabel(
          context,
          { key: edge, ...edgeData, size: sigma.scaleSize(edgeData.size) },
          { key: source, ...sourceData, ...sigma.framedGraphToViewport(sourceData), size: sigma.scaleSize(sourceData.size) },
          { key: target, ...targetData, ...sigma.framedGraphToViewport(targetData), size: sigma.scaleSize(targetData.size) },
          settings,
        );
      });

      activeNodes.forEach((active) => {
        const data = sigma.getNodeDisplayData(active);
        if (!data || !data.label || data.hidden) return;
        drawDiscNodeLabel(
          context,
          { key: active, ...data, ...sigma.framedGraphToViewport(data), size: sigma.scaleSize(data.size) },
          settings,
        );
      });
    };

    sigma.on("afterRender", draw);
    draw();
    return () => {
      sigma.off("afterRender", draw);
      killLayer();
    };
  }, [sigma]);
}
