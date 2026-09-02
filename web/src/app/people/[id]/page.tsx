import Link from "next/link";
import { ArrowLeft, GitFork } from "lucide-react";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ContactOverview } from "@/components/contact-overview";
import { InteractionTimeline } from "@/components/interaction-timeline";
import { RelationshipIntelligence } from "@/components/relationship-intelligence";
import { ScoreBreakdown } from "@/components/score-breakdown";
import { getPerson } from "@/lib/crm";
import { initials, titleCase } from "@/lib/format";

export default async function PersonPage({ params }: PageProps<"/people/[id]">) {
  const { id } = await params;
  const detail = await getPerson(id);
  const { person, score } = detail;

  return <>
    <header className="border-b px-5 py-6 sm:px-8 lg:px-10">
      <Button asChild variant="ghost" size="sm" className="mb-5 -ml-2"><Link href="/"><ArrowLeft />People</Link></Button>
      <div className="flex flex-col gap-5 lg:flex-row lg:items-end lg:justify-between">
        <div className="flex min-w-0 items-center gap-4">
          <Avatar className="size-16"><AvatarFallback className="text-lg">{initials(person.display_name)}</AvatarFallback></Avatar>
          <div className="min-w-0"><h1 className="truncate text-3xl font-semibold tracking-tight sm:text-4xl">{person.display_name}</h1><div className="mt-2 flex flex-wrap gap-2"><Badge>{titleCase(person.affinity_tier)}</Badge><Badge variant="secondary">{titleCase(person.activity_state)}</Badge><Badge variant="outline">Affinity {score.affinity_score.toFixed(1)}</Badge>{person.lifecycle_state === "retired" && <Badge variant="destructive">Retired</Badge>}</div></div>
        </div>
        <Button asChild variant="outline"><Link href={`/network?person=${encodeURIComponent(person.id)}`}><GitFork />Find in network</Link></Button>
      </div>
    </header>
    <div className="space-y-4 px-5 py-7 sm:px-8 lg:px-10">
      <ScoreBreakdown score={score} />
      <Tabs defaultValue="overview">
        <TabsList variant="line" className="mb-4 w-full justify-start overflow-x-auto"><TabsTrigger value="overview">Overview</TabsTrigger><TabsTrigger value="timeline">Timeline</TabsTrigger><TabsTrigger value="relationships">Relationships</TabsTrigger></TabsList>
        <TabsContent value="overview"><ContactOverview detail={detail} /></TabsContent>
        <TabsContent value="timeline"><InteractionTimeline interactions={detail.interactions} /></TabsContent>
        <TabsContent value="relationships"><RelationshipIntelligence detail={detail} /></TabsContent>
      </Tabs>
    </div>
  </>;
}
