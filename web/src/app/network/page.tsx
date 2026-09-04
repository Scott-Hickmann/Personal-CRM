import { NetworkGraphLoader } from "@/components/network-graph-loader";
import { PageHeader } from "@/components/page-header";
import { getOverview } from "@/lib/crm";

export default async function NetworkPage({ searchParams }: PageProps<"/network">) {
  const [{ person }, overview] = await Promise.all([searchParams, getOverview()]);
  return <>
    <PageHeader eyebrow="Shared conversations" title="Relationship network" description="Explore observed connections, then narrow them by person, affinity, or activity." />
    <NetworkGraphLoader overview={overview} focusedPerson={typeof person === "string" ? person : undefined} />
  </>;
}
