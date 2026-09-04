import Link from "next/link";
import { ArrowUpRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatDate, titleCase } from "@/lib/format";
import type { PersonDetail } from "@/lib/types";

export function RelationshipIntelligence({ detail }: { detail: PersonDetail }) {
  return <div className="grid gap-4 xl:grid-cols-[1fr_0.8fr]">
    <Card><CardHeader><CardTitle>Relationships</CardTitle></CardHeader><CardContent className="space-y-2">{detail.relationships.length ? detail.relationships.map((item) => <Link key={item.id} href={`/people/${encodeURIComponent(item.person_id)}`} className="hover:bg-muted/60 flex items-start justify-between gap-4 rounded-lg border p-3 transition-colors"><div><div className="flex flex-wrap items-center gap-2"><span className="font-medium">{item.display_name}</span><Badge variant="secondary">{titleCase(item.relationship_type)}</Badge><Badge variant="outline">{Math.round(item.classification_confidence * 100)}% classification</Badge></div><p className="text-muted-foreground mt-2 text-xs">{item.shared_context_count} shared {item.shared_context_count === 1 ? "conversation" : "conversations"} · Last observed {formatDate(item.last_observed_at)}</p><Evidence value={item.classification_evidence} /></div><ArrowUpRight className="text-muted-foreground mt-1 size-4 shrink-0" /></Link>) : <p className="text-muted-foreground py-8 text-center text-sm">No shared-conversation relationships yet.</p>}</CardContent></Card>
    <Card><CardHeader><CardTitle>Semantic summaries</CardTitle></CardHeader><CardContent className="space-y-3">{detail.summaries.length ? detail.summaries.map((summary) => <article key={summary.id} className="bg-muted/60 rounded-lg p-4"><p className="whitespace-pre-wrap text-sm leading-6">{summary.summary}</p><p className="text-muted-foreground mt-3 font-mono text-xs">{summary.model_version} · {formatDate(summary.created_at)}</p></article>) : <p className="text-muted-foreground py-8 text-center text-sm">No summaries generated yet.</p>}</CardContent></Card>
  </div>;
}

function Evidence({ value }: { value: unknown }) {
  if (!value || (Array.isArray(value) && value.length === 0)) return null;
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return <details className="mt-2"><summary className="text-muted-foreground cursor-pointer text-xs">View evidence</summary><pre className="bg-muted mt-2 max-h-40 overflow-auto rounded-md p-2 text-[0.7rem] whitespace-pre-wrap">{text}</pre></details>;
}
