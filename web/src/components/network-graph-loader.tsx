"use client";

import dynamic from "next/dynamic";
import type { Overview } from "@/lib/types";

const ClientNetworkGraph = dynamic(
  () => import("@/components/network-graph").then((module) => module.NetworkGraph),
  {
    loading: () => <div className="h-[calc(100vh-10rem)] min-h-[44rem] animate-pulse bg-muted/20" />,
    ssr: false,
  },
);

export function NetworkGraphLoader(props: { overview: Overview; focusedPerson?: string }) {
  return <ClientNetworkGraph {...props} />;
}
