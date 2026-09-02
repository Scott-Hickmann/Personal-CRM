import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { relativeDays } from "@/lib/format";
import type { Score } from "@/lib/types";

export function ScoreBreakdown({ score }: { score: Score }) {
  const measures = [
    ["Behavioral", score.behavioral_score],
    ["Semantic", score.semantic_score],
    ["Overall affinity", score.affinity_score],
  ] as const;
  return (
    <Card>
      <CardHeader><CardTitle>Closeness model</CardTitle></CardHeader>
      <CardContent className="grid gap-6 lg:grid-cols-[1fr_1fr]">
        <div className="space-y-4">{measures.map(([label, value]) => <div key={label}><div className="mb-1.5 flex items-center justify-between text-xs"><span className="text-muted-foreground">{label}</span><span className="font-mono">{value.toFixed(1)}</span></div><div className="bg-muted h-2 overflow-hidden rounded-full"><div className="bg-primary h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /></div></div>)}</div>
        <dl className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-3">
          <Stat label="90d interactions" value={score.components.interactions_90d} />
          <Stat label="Active days" value={score.components.active_days_90d} />
          <Stat label="Channels" value={score.components.channels_90d} />
          <Stat label="Incoming" value={score.components.incoming_90d} />
          <Stat label="Outgoing" value={score.components.outgoing_90d} />
          <Stat label="Last seen" value={relativeDays(score.components.days_since_last)} />
        </dl>
      </CardContent>
    </Card>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) { return <div><dt className="text-muted-foreground text-xs">{label}</dt><dd className="mt-1 font-mono text-base">{value}</dd></div>; }
