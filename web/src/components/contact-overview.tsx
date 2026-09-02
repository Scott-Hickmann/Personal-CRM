import { CalendarDays, Camera, Clock3, Hash, Mail, Phone, StickyNote } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { MutationForms } from "@/components/mutation-forms";
import { formatDate, titleCase } from "@/lib/format";
import type { PersonDetail } from "@/lib/types";

export function ContactOverview({ detail }: { detail: PersonDetail }) {
  const { person } = detail;
  return (
    <div className="grid gap-4 xl:grid-cols-[1.25fr_0.75fr]">
      <div className="space-y-4">
        <Card>
          <CardHeader><CardTitle>Contact record</CardTitle></CardHeader>
          <CardContent className="space-y-4">
            <dl className="grid gap-3 sm:grid-cols-2">
              {person.identities.map((identity) => (
                <div key={`${identity.kind}-${identity.value}`} className="flex items-start gap-2">
                  {identity.kind === "email" ? <Mail className="text-muted-foreground mt-0.5 size-4" /> : <Phone className="text-muted-foreground mt-0.5 size-4" />}
                  <div><dt className="text-muted-foreground text-xs">{titleCase(identity.kind)}</dt><dd className="break-all text-sm">{identity.value}</dd></div>
                </div>
              ))}
            </dl>
            <div className="flex flex-wrap gap-1.5">{person.tags.length ? person.tags.map((tag) => <Badge key={tag} variant="secondary"><Hash className="size-3" />{tag}</Badge>) : <p className="text-muted-foreground text-sm">No tags yet.</p>}</div>
            <div className="text-muted-foreground grid gap-2 border-t pt-4 font-mono text-xs sm:grid-cols-2">
              <p>CRM ID <span className="text-foreground block truncate">{person.id}</span></p>
              <p>iCloud ID <span className="text-foreground block truncate">{person.apple_contact_id ?? "Not linked"}</span></p>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Notes & facts</CardTitle></CardHeader>
          <CardContent className="grid gap-5 lg:grid-cols-2">
            <section><h3 className="mb-3 flex items-center gap-2 text-sm font-medium"><StickyNote className="size-4" />Notes</h3><div className="space-y-2">{person.notes.length ? person.notes.map((note) => <div key={note.id} className="bg-muted/60 rounded-lg p-3"><p className="whitespace-pre-wrap text-sm leading-6">{note.body}</p><p className="text-muted-foreground mt-2 text-xs">{formatDate(note.created_at)}</p></div>) : <Empty text="No notes yet." />}</div></section>
            <section><h3 className="mb-3 text-sm font-medium">Facts</h3><dl className="divide-y">{person.facts.length ? person.facts.map((fact) => <div key={fact.key} className="grid grid-cols-[7rem_1fr] gap-3 py-2 text-sm"><dt className="text-muted-foreground truncate">{titleCase(fact.key)}</dt><dd className="break-words">{fact.value}</dd></div>) : <Empty text="No facts yet." />}</dl></section>
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Follow-ups & dates</CardTitle></CardHeader>
          <CardContent className="grid gap-5 lg:grid-cols-2">
            <section><h3 className="mb-3 flex items-center gap-2 text-sm font-medium"><Clock3 className="size-4" />Follow-ups</h3><div className="space-y-2">{detail.followups.length ? detail.followups.map((item) => <div key={item.id} className="rounded-lg border p-3"><p className={item.completed_at ? "text-muted-foreground line-through" : ""}>{item.body}</p><p className="text-muted-foreground mt-1 text-xs">{item.completed_at ? `Completed ${formatDate(item.completed_at)}` : item.due_at ? `Due ${formatDate(item.due_at, { dateStyle: "medium", timeStyle: "short" })}` : "No due date"}</p></div>) : <Empty text="No follow-ups yet." />}</div></section>
            <section><h3 className="mb-3 flex items-center gap-2 text-sm font-medium"><CalendarDays className="size-4" />Important dates</h3><div className="space-y-2">{detail.important_dates.length ? detail.important_dates.map((item) => <div key={item.id} className="flex items-center justify-between gap-3 rounded-lg border px-3 py-2"><span>{item.label}</span><span className="text-muted-foreground text-xs">{item.date}{item.recurring ? " · yearly" : ""}</span></div>) : <Empty text="No important dates yet." />}</div></section>
          </CardContent>
        </Card>
      </div>

      <div className="space-y-4">
        <Card>
          <CardHeader><CardTitle>Add CRM context</CardTitle></CardHeader>
          <CardContent><MutationForms personId={person.id} closenessRating={detail.score.closeness_rating} /></CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>Cadence & Photos</CardTitle></CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="flex items-start gap-3"><Clock3 className="text-muted-foreground mt-0.5 size-4" /><div><p className="font-medium">Contact cadence</p><p className="text-muted-foreground">{detail.cadence ? `Every ${detail.cadence.interval_days} days` : "No cadence configured"}</p></div></div>
            <div className="flex items-start gap-3"><Camera className="text-muted-foreground mt-0.5 size-4" /><div><p className="font-medium">Photos link</p><p className="text-muted-foreground">{detail.photo ? `${titleCase(detail.photo.state)}${detail.photo.photos_name ? ` · ${detail.photo.photos_name}` : ""}` : "No Photos link"}</p>{detail.photo?.photos_asset_id && <p className="text-muted-foreground mt-1 font-mono text-xs">{detail.photo.photos_asset_id}</p>}</div></div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function Empty({ text }: { text: string }) { return <p className="text-muted-foreground py-3 text-sm">{text}</p>; }
