import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { relativeDays } from "@/lib/format";
import type { Score } from "@/lib/types";

export function ScoreBreakdown({ score }: { score: Score }) {
  const measures = [
    ["Behavioral evidence", score.behavioral_score],
    ["Calibrated affinity", score.affinity_score],
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
            <Stat label="Relationship span" value={`${Math.round(score.components.relationship_span_days)}d`} />
          </dl>
        </div>
      </CardContent>
    </Card>
  );
}

function Measure({ label, value }: { label: string; value: number }) {
  return <div><div className="mb-1.5 flex items-center justify-between text-xs"><span className="text-muted-foreground">{label}</span><span className="font-mono">{value.toFixed(1)}</span></div><div className="bg-muted h-2 overflow-hidden rounded-full"><div className="bg-primary h-full rounded-full" style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /></div></div>;
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return <div><dt className="text-muted-foreground text-xs">{label}</dt><dd className="mt-1 font-mono text-base">{value}</dd></div>;
}
