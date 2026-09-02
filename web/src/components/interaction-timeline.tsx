"use client";

import { useMemo, useState } from "react";
import { ChevronDown, ChevronUp, FileText, LoaderCircle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { formatDate, titleCase } from "@/lib/format";
import type { InteractionBody, InteractionPreview } from "@/lib/types";

export function InteractionTimeline({ interactions }: { interactions: InteractionPreview[] }) {
  const [channel, setChannel] = useState("all");
  const channels = useMemo(() => [...new Set(interactions.map((item) => item.channel))].sort(), [interactions]);
  const visible = channel === "all" ? interactions : interactions.filter((item) => item.channel === channel);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-muted-foreground text-sm">{visible.length} interactions</p>
        <Select value={channel} onValueChange={setChannel}><SelectTrigger className="w-44"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="all">All channels</SelectItem>{channels.map((value) => <SelectItem key={value} value={value}>{titleCase(value)}</SelectItem>)}</SelectContent></Select>
      </div>
      {visible.length ? <ol className="relative space-y-3 before:absolute before:inset-y-4 before:left-[0.45rem] before:w-px before:bg-border">
        {visible.map((interaction) => <TimelineItem key={interaction.id} interaction={interaction} />)}
      </ol> : <p className="text-muted-foreground py-10 text-center text-sm">No interactions in this channel.</p>}
    </div>
  );
}

function TimelineItem({ interaction }: { interaction: InteractionPreview }) {
  const [body, setBody] = useState<InteractionBody | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  async function reveal() {
    if (body) { setBody(null); return; }
    setLoading(true); setError("");
    try {
      const response = await fetch(`/api/interactions/${encodeURIComponent(interaction.id)}`, { cache: "no-store" });
      if (!response.ok) throw new Error("Could not reveal this interaction.");
      setBody(await response.json());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not reveal this interaction.");
    } finally { setLoading(false); }
  }

  return (
    <li className="relative pl-7">
      <span className="bg-background border-primary absolute top-5 left-0 size-4 rounded-full border-[3px]" />
      <div className="bg-card rounded-xl border p-4">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="flex flex-wrap items-center gap-2"><Badge variant="secondary">{titleCase(interaction.channel)}</Badge><Badge variant="outline">{titleCase(interaction.direction)}</Badge><span className="text-muted-foreground text-xs">{formatDate(interaction.occurred_at, { dateStyle: "medium", timeStyle: "short" })}</span></div>
          {interaction.has_body && <Button variant="ghost" size="sm" onClick={reveal} disabled={loading}>{loading ? <LoaderCircle className="animate-spin" /> : body ? <ChevronUp /> : <ChevronDown />}{body ? "Hide" : "Reveal"}</Button>}
        </div>
        {interaction.subject && <p className="mt-3 text-sm font-medium">{interaction.subject}</p>}
        {body ? <p className="mt-3 whitespace-pre-wrap text-sm leading-6">{body.body || "No message body stored."}</p> : interaction.preview && <p className="text-muted-foreground mt-3 line-clamp-2 text-sm leading-6">{interaction.preview}</p>}
        {interaction.attachments.length > 0 && <div className="text-muted-foreground mt-3 flex flex-wrap gap-2 text-xs">{interaction.attachments.map((attachment, index) => <span key={`${attachment.filename}-${index}`} className="bg-muted flex items-center gap-1 rounded-md px-2 py-1"><FileText className="size-3" />{attachment.filename ?? attachment.mime_type ?? "Attachment"}</span>)}</div>}
        {error && <p className="text-destructive mt-2 text-xs">{error}</p>}
      </div>
    </li>
  );
}
