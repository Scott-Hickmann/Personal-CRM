import { NetworkGraphLoader } from "@/components/network-graph-loader";
import { PageHeader } from "@/components/page-header";
import { getOverview, runCrm } from "@/lib/crm";
import type { ClusterLevel } from "@/lib/clusters";

export default async function NetworkPage({ searchParams }: PageProps<"/network">) {
  const [{ person }, overview, clusters] = await Promise.all([searchParams, getOverview(), runCrm<ClusterLevel[]>(["cluster", "list"])]);
  return <>
    <PageHeader eyebrow="Shared conversations" title="Relationship network" description="Explore observed connections, then narrow them by person, affinity, or activity." />
    <NetworkGraphLoader overview={overview} clusterLevels={clusters} focusedPerson={typeof person === "string" ? person : undefined} />
  </>;
}
