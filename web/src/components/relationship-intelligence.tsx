import Link from "next/link";
import { ArrowUpRight } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatDate } from "@/lib/format";
import type { PersonDetail } from "@/lib/types";

export function RelationshipIntelligence({ detail }: { detail: PersonDetail }) {
  return <Card><CardHeader><CardTitle>Shared-conversation relationships</CardTitle></CardHeader><CardContent className="space-y-2">{detail.relationships.length ? detail.relationships.map((item) => <Link key={item.id} href={`/people/${encodeURIComponent(item.person_id)}`} className="hover:bg-muted/60 flex items-start justify-between gap-4 rounded-lg border p-3 transition-colors"><div><span className="font-medium">{item.display_name}</span><p className="text-muted-foreground mt-2 text-xs">{item.shared_context_count} shared {item.shared_context_count === 1 ? "conversation" : "conversations"} · Last observed {formatDate(item.last_observed_at)}</p></div><ArrowUpRight className="text-muted-foreground mt-1 size-4 shrink-0" /></Link>) : <p className="text-muted-foreground py-8 text-center text-sm">No shared-conversation relationships yet.</p>}</CardContent></Card>;
}
