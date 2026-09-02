import { PeopleExplorer } from "@/components/people-explorer";
import { PageHeader } from "@/components/page-header";
import { getOverview } from "@/lib/crm";

export default async function PeoplePage() {
  const overview = await getOverview();
  return (
    <>
      <PageHeader
        eyebrow="Relationship index"
        title="People"
        description="Search every contact and inspect the context, history, and signals your local CRM has collected."
      />
      <PeopleExplorer people={overview.people} relationshipCount={overview.graph.edges.length} />
    </>
  );
}
