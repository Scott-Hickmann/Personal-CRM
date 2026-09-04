"use client";

import { useState, useTransition } from "react";
import Link from "next/link";
import { setClusterName } from "@/app/cluster-actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { Cluster, ClusterLevel } from "@/lib/clusters";
import type { Overview } from "@/lib/types";

export function NetworkClusters({ level, clusters, selected, onSelect, onRename, people, visiblePeople }: {
  level: ClusterLevel; clusters: Cluster[]; selected?: string;
  onSelect: (id?: string) => void; onRename: (id: string, name: string | null) => void;
  people: Overview["people"]; visiblePeople: Set<string>;
}) {
  const personMap = new Map(people.map((p) => [p.id, p.display_name]));
  const ordered = [...clusters].sort((a,b) => b.members.length - a.members.length || a.name.localeCompare(b.name));
  const active = clusters.find((c) => c.id === selected);
  const bridges = level.bridges.filter((b) => !active || b.primary_cluster === active.id || b.secondary_cluster === active.id);
  return <aside className="space-y-4 rounded-xl border bg-card p-4 text-sm lg:max-h-[calc(100vh-20rem)] lg:overflow-y-auto" aria-label="Network clusters">
    <div className="flex items-center justify-between"><h2 className="font-semibold">{clusters.length} communities</h2>{selected && <button className="underline" onClick={() => onSelect()}>Clear cluster</button>}</div>
    <p className="text-muted-foreground text-xs">Shared-conversation groups. Names are suggestions from recorded evidence.</p>
    <div className="max-h-64 space-y-1 overflow-y-auto">{ordered.map((cluster) => {
      const visible = cluster.members.filter((id) => visiblePeople.has(id)).length;
      return <button key={cluster.id} aria-pressed={selected === cluster.id} disabled={!visible}
        className={`flex w-full items-start gap-2 rounded-md p-2 text-left disabled:opacity-40 ${selected === cluster.id ? "bg-accent" : "hover:bg-accent/50"}`}
        onClick={() => onSelect(selected === cluster.id ? undefined : cluster.id)}>
        <span className="mt-1 size-2.5 shrink-0 rounded-full" style={{ backgroundColor: cluster.color }} />
        <span className="min-w-0 flex-1 break-words">{cluster.name}</span>
        <span className="text-muted-foreground text-xs">{visible === cluster.members.length ? visible : `${visible}/${cluster.members.length}`}</span>
      </button>;
    })}</div>
    {active && <div className="space-y-3 border-t pt-3">
      <ClusterName key={active.id} cluster={active} onRename={onRename} />
      <details open><summary className="cursor-pointer font-medium">Why this group?</summary>
        {active.evidence.length ? <ul className="mt-2 space-y-2 text-xs">{active.evidence.map((e) => <li key={e.source}>
          <span className="font-medium">{e.label}</span><span className="text-muted-foreground"> · {e.kind} · {e.member_count} members ({Math.round(e.coverage * 100)}% of group)</span>
        </li>)}</ul> : <p className="text-muted-foreground mt-2 text-xs">No shared title, tag, or organization supports a specific name. The suggested name uses representative members.</p>}
        {active.predecessors.length > 0 && <p className="text-muted-foreground mt-2 text-xs">Changed membership from {active.predecessors.map((p) => p.name).join(", ")}. Previous custom names were preserved separately; rename this group if needed.</p>}
      </details>
      <details><summary className="cursor-pointer">Members ({active.members.length})</summary><ul className="mt-2 space-y-1">{active.members.map((id) => <li key={id}><Link className="underline" href={`/people/${encodeURIComponent(id)}`}>{personMap.get(id) ?? "Unknown person"}</Link></li>)}</ul></details>
    </div>}
    {bridges.length > 0 && <details><summary className="cursor-pointer">People connecting groups ({new Set(bridges.map((b) => b.person_id)).size})</summary>
      <ul className="mt-2 space-y-2 text-xs">{bridges.map((b) => <li key={`${b.person_id}:${b.secondary_cluster}`}>
        <Link className="underline" href={`/people/${encodeURIComponent(b.person_id)}`}>{personMap.get(b.person_id) ?? "Unknown person"}</Link>
        <span className="text-muted-foreground"> → {clusters.find((c) => c.id === b.secondary_cluster)?.name} ({Math.round(b.external_share * 100)}% of connection weight)</span>
      </li>)}</ul>
    </details>}
    <details className="border-t pt-3 text-xs text-muted-foreground"><summary className="cursor-pointer">Clustering quality</summary>
      <p className="mt-2">Repeat-run agreement: {Math.round(level.seed_agreement * 100)}%. Within-group connection weight: {Math.round(level.internal_weight_share * 100)}%.</p>
      <p className="mt-2">Without large-chat discounting: {level.raw_cluster_count} groups, {Math.round(level.raw_weight_agreement * 100)}% membership agreement.</p>
      <p className="mt-2">These measure structure and stability, not whether the group names are correct. Membership stays fixed while searching and filtering.</p>
    </details>
  </aside>;
}

function ClusterName({ cluster, onRename }: { cluster: Cluster; onRename: (id: string, name: string | null) => void }) {
  const [name, setName] = useState(cluster.name);
  const [message, setMessage] = useState("");
  const [pending, startTransition] = useTransition();
  function save(value: string | null) {
    startTransition(async () => {
      try {
        const result = await setClusterName(cluster.id, value);
        setMessage(result.message);
        if (result.status === "success") {
          setName(value?.trim() ?? cluster.suggested_name);
          onRename(cluster.id, value);
        }
      } catch { setMessage("Unable to save. Please try again."); }
    });
  }
  return <form onSubmit={(event) => { event.preventDefault(); save(name.trim()); }} className="space-y-2">
    <label className="font-medium" htmlFor="cluster-name">Cluster name</label>
    <Input id="cluster-name" maxLength={80} required value={name} onChange={(event) => setName(event.target.value)} />
    <div className="flex flex-wrap gap-2"><Button size="sm" disabled={pending || !name.trim()}>{pending ? "Saving…" : "Save name"}</Button>
      {cluster.custom_name && <Button size="sm" variant="outline" type="button" disabled={pending} onClick={() => save(null)}>Use suggestion</Button>}</div>
    {message && <p role="status" className="text-xs">{message}</p>}
  </form>;
}
