import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatDate, relativeDays } from "@/lib/format";
import type { Score } from "@/lib/types";

export function ScoreBreakdown({ score }: { score: Score }) {
  const relational = score.components.relational;
  const measures = [
    ["Behavioral evidence", score.behavioral_score],
    ["Ollama relational evidence", score.relational_score],
    ["Calibrated affinity", score.affinity_score],
  ] as const;
  const dimensions = [
    ["Intimacy", relational.intimacy],
    ["Emotional support", relational.emotional_support],
    ["Practical support", relational.practical_support],
    ["Affection", relational.affection],
    ["Shared activity", relational.shared_activity],
    ["Conflict repair", relational.conflict_repair],
  ] as const;

  return (
    <Card>
      <CardHeader><CardTitle>Closeness model</CardTitle></CardHeader>
      <CardContent className="space-y-6">
        <div className="grid gap-6 lg:grid-cols-[1fr_1fr]">
          <div className="space-y-4">
            {measures.map(([label, value]) => <Measure key={label} label={label} value={value} />)}
            <p className="text-muted-foreground text-xs">
              {score.calibration.rating_count >= 5
                ? `Personalized with ${score.calibration.rating_count} closeness ratings.`
                : score.calibration.rating_count > 0
                  ? `${score.calibration.rating_count}/5 ratings added; rated people are anchored directly and global calibration starts at five.`
                  : "Using the uncalibrated prior. Add closeness ratings to personalize it."}
              {score.closeness_rating ? ` Your rating: ${score.closeness_rating}/7.` : ""}
            </p>
          </div>
          <dl className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-3">
            <Stat label="30d interactions" value={score.components.interactions_30d} />
            <Stat label="90d interactions" value={score.components.interactions_90d} />
            <Stat label="365d interactions" value={score.components.interactions_365d} />
            <Stat label="Active weeks" value={score.components.active_weeks_90d} />
            <Stat label="Channels" value={score.components.channels_90d} />
            <Stat label="Last seen" value={relativeDays(score.components.days_since_last)} />
            <Stat label="Model assessed" value={relational.assessed_interactions} />
            <Stat label="Meaningful evidence" value={relational.meaningful_interactions} />
            <Stat label="Relationship span" value={`${Math.round(score.components.relationship_span_days)}d`} />
          </dl>
        </div>

        <div className="border-t pt-5">
          <h3 className="mb-4 text-sm font-medium">Relational dimensions</h3>
          <div className="grid gap-x-6 gap-y-3 md:grid-cols-2">{dimensions.map(([label, value]) => <Measure key={label} label={label} value={value} compact />)}</div>
        </div>

        {relational.evidence.length > 0 && <div className="border-t pt-5"><h3 className="mb-3 text-sm font-medium">Recent supporting evidence</h3><ul className="space-y-2">{relational.evidence.map((item, index) => <li key={`${item.occurred_at}-${index}`} className="bg-muted/60 rounded-lg p-3 text-sm"><p>{item.summary}</p><p className="text-muted-foreground mt-1 text-xs">{formatDate(item.occurred_at)}</p></li>)}</ul></div>}
      </CardContent>
    </Card>
  );
}

function Measure({ label, value, compact = false }: { label: string; value: number; compact?: boolean }) {
  return <div><div className="mb-1.5 flex items-center justify-between text-xs"><span className="text-muted-foreground">{label}</span><span className="font-mono">{value.toFixed(1)}</span></div><div className={`bg-muted overflow-hidden rounded-full ${compact ? "h-1.5" : "h-2"}`}><div className="bg-primary h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /></div></div>;
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return <div><dt className="text-muted-foreground text-xs">{label}</dt><dd className="mt-1 font-mono text-base">{value}</dd></div>;
}
