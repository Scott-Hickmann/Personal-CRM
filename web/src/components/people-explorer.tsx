"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { ArrowUpRight, MessageSquareText, Search, UserRoundCheck } from "lucide-react";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { formatDate, initials, titleCase } from "@/lib/format";
import type { OverviewPerson } from "@/lib/types";

export function PeopleExplorer({ people, relationshipCount }: { people: OverviewPerson[]; relationshipCount: number }) {
  const [query, setQuery] = useState("");
  const [tier, setTier] = useState("all");
  const [activity, setActivity] = useState("all");
  const [lifecycle, setLifecycle] = useState("active");
  const [shown, setShown] = useState(60);

  const visible = useMemo(() => {
    const term = query.trim().toLocaleLowerCase();
    return people.filter((person) => {
      const searchable = [person.display_name, ...person.identities, ...person.tags].join(" ").toLocaleLowerCase();
      return (!term || searchable.includes(term))
        && (tier === "all" || person.affinity_tier === tier)
        && (activity === "all" || person.activity_state === activity)
        && (lifecycle === "all" || person.lifecycle_state === lifecycle)
        && !person.is_self;
    });
  }, [activity, lifecycle, people, query, tier]);

  const activePeople = people.filter((person) => person.lifecycle_state === "active" && !person.is_self).length;
  const warmPeople = people.filter((person) => ["core", "close"].includes(person.affinity_tier ?? "")).length;

  return (
    <div className="space-y-6 px-5 py-7 sm:px-8 lg:px-10">
      <section className="grid gap-3 sm:grid-cols-3">
        <Metric icon={UserRoundCheck} label="Active people" value={activePeople} />
        <Metric icon={MessageSquareText} label="Core + close" value={warmPeople} />
        <Metric icon={ArrowUpRight} label="Relationship links" value={relationshipCount} />
      </section>

      <Card>
        <CardContent className="grid gap-3 p-4 lg:grid-cols-[minmax(16rem,1fr)_11rem_11rem_11rem]">
          <label className="relative">
            <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search names, emails, phones, or tags…"
              className="pl-9"
              autoFocus
            />
          </label>
          <Filter value={tier} onChange={setTier} label="All tiers" options={["core", "close", "familiar", "acquaintance", "peripheral"]} />
          <Filter value={activity} onChange={setActivity} label="All activity" options={["active", "cooling", "dormant", "never"]} />
          <Filter value={lifecycle} onChange={setLifecycle} label="All records" options={["active", "retired"]} />
        </CardContent>
      </Card>

      <div className="flex items-center justify-between">
        <p className="text-muted-foreground text-sm">{visible.length} {visible.length === 1 ? "person" : "people"}</p>
        <p className="text-muted-foreground hidden text-xs sm:block">Select a person to inspect the complete CRM record</p>
      </div>

      {visible.length ? (
        <section className="grid gap-3 xl:grid-cols-2">
          {visible.slice(0, shown).map((person) => <PersonCard key={person.id} person={person} />)}
        </section>
      ) : (
        <Card className="border-dashed"><CardContent className="py-16 text-center">
          <Search className="text-muted-foreground mx-auto mb-3 size-6" />
          <p className="font-medium">No matching people</p>
          <p className="text-muted-foreground mt-1 text-sm">Try removing a filter or searching another identity.</p>
        </CardContent></Card>
      )}
      {shown < visible.length && <div className="flex justify-center"><Button variant="outline" onClick={() => setShown((value) => value + 60)}>Show 60 more</Button></div>}
    </div>
  );
}

function PersonCard({ person }: { person: OverviewPerson }) {
  return (
    <Link href={`/people/${encodeURIComponent(person.id)}`} className="group block">
      <Card className="h-full transition-colors group-hover:border-foreground/25">
        <CardHeader className="flex-row items-start justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <Avatar className="size-11"><AvatarFallback>{initials(person.display_name)}</AvatarFallback></Avatar>
            <div className="min-w-0">
              <h2 className="truncate font-medium">{person.display_name}</h2>
              <p className="text-muted-foreground truncate text-xs">{person.identities[0] ?? "No active identity"}</p>
            </div>
          </div>
          <ArrowUpRight className="text-muted-foreground size-4 shrink-0 transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5" />
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-1.5">
            <Badge variant="secondary">{titleCase(person.affinity_tier)}</Badge>
            <Badge variant="outline">{titleCase(person.activity_state)}</Badge>
            {person.lifecycle_state === "retired" && <Badge variant="destructive">Retired</Badge>}
            {person.tags.slice(0, 3).map((tag) => <Badge key={tag} variant="outline">#{tag}</Badge>)}
          </div>
          <div className="text-muted-foreground grid grid-cols-3 gap-3 text-xs">
            <div><span className="text-foreground block font-mono text-base">{person.affinity_score?.toFixed(0) ?? "—"}</span>Affinity</div>
            <div><span className="text-foreground block font-mono text-base">{person.interaction_count}</span>Interactions</div>
            <div><span className="text-foreground block truncate text-sm">{formatDate(person.last_interaction_at, { month: "short", day: "numeric" })}</span>Last contact</div>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

function Metric({ icon: Icon, label, value }: { icon: typeof UserRoundCheck; label: string; value: number }) {
  return <Card><CardContent className="flex items-center gap-3 p-4"><span className="bg-muted grid size-9 place-items-center rounded-lg"><Icon className="size-4" /></span><div><p className="font-mono text-xl font-semibold">{value}</p><p className="text-muted-foreground text-xs">{label}</p></div></CardContent></Card>;
}

function Filter({ value, onChange, label, options }: { value: string; onChange: (value: string) => void; label: string; options: string[] }) {
  return <Select value={value} onValueChange={onChange}><SelectTrigger className="w-full"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">{label}</SelectItem>{options.map((option) => <SelectItem key={option} value={option}>{titleCase(option)}</SelectItem>)}</SelectContent></Select>;
}
