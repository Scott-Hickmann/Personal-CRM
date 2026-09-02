"use client";

import { useActionState } from "react";
import { addFollowupAction, addNoteAction, addTagAction, clearAffinityAction, setAffinityAction, setFactAction } from "@/app/actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { ActionState } from "@/lib/types";

const initial: ActionState = { status: "idle", message: "" };

export function MutationForms({ personId, closenessRating }: { personId: string; closenessRating: number | null }) {
  const [note, noteAction, notePending] = useActionState(addNoteAction, initial);
  const [fact, factAction, factPending] = useActionState(setFactAction, initial);
  const [tag, tagAction, tagPending] = useActionState(addTagAction, initial);
  const [followup, followupAction, followupPending] = useActionState(addFollowupAction, initial);
  const [affinity, affinityAction, affinityPending] = useActionState(setAffinityAction, initial);
  const [clearAffinity, clearAffinityActionWithState, clearAffinityPending] = useActionState(clearAffinityAction, initial);

  return (
    <Tabs defaultValue="note">
      <TabsList className="w-full justify-start overflow-x-auto">
        <TabsTrigger value="note">Note</TabsTrigger>
        <TabsTrigger value="fact">Fact</TabsTrigger>
        <TabsTrigger value="tag">Tag</TabsTrigger>
        <TabsTrigger value="followup">Follow-up</TabsTrigger>
        <TabsTrigger value="affinity">Closeness</TabsTrigger>
      </TabsList>
      <TabsContent value="note"><form action={noteAction} className="space-y-3"><Person id={personId} /><Label htmlFor="note-text">New note</Label><Textarea id="note-text" name="text" required placeholder="What should you remember?" /><Submit pending={notePending} label="Add note" /><Status state={note} /></form></TabsContent>
      <TabsContent value="fact"><form action={factAction} className="space-y-3"><Person id={personId} /><div className="grid gap-3 sm:grid-cols-2"><div className="space-y-2"><Label htmlFor="fact-key">Key</Label><Input id="fact-key" name="key" required placeholder="birthday" /></div><div className="space-y-2"><Label htmlFor="fact-value">Value</Label><Input id="fact-value" name="value" required placeholder="May 4" /></div></div><Submit pending={factPending} label="Save fact" /><Status state={fact} /></form></TabsContent>
      <TabsContent value="tag"><form action={tagAction} className="space-y-3"><Person id={personId} /><Label htmlFor="tag-value">Tag</Label><Input id="tag-value" name="tag" required placeholder="friend" /><Submit pending={tagPending} label="Add tag" /><Status state={tag} /></form></TabsContent>
      <TabsContent value="followup"><form action={followupAction} className="space-y-3"><Person id={personId} /><Label htmlFor="followup-text">Follow-up</Label><Textarea id="followup-text" name="text" required placeholder="Check in about…" /><div className="space-y-2"><Label htmlFor="followup-due">Due (optional)</Label><Input id="followup-due" type="datetime-local" name="due" /></div><Submit pending={followupPending} label="Add follow-up" /><Status state={followup} /></form></TabsContent>
      <TabsContent value="affinity" className="space-y-3">
        <form action={affinityAction} className="space-y-3">
          <Person id={personId} />
          <Label htmlFor="affinity-rating">Your closeness rating</Label>
          <select id="affinity-rating" name="rating" defaultValue={closenessRating ?? 4} className="border-input bg-background h-9 w-full rounded-md border px-3 text-sm">
            <option value="1">1 — barely know</option>
            <option value="2">2 — weak tie</option>
            <option value="3">3 — acquaintance</option>
            <option value="4">4 — familiar</option>
            <option value="5">5 — friend</option>
            <option value="6">6 — close</option>
            <option value="7">7 — inner circle</option>
          </select>
          <Submit pending={affinityPending} label="Save rating" />
          <Status state={affinity} />
        </form>
        {closenessRating !== null && <form action={clearAffinityActionWithState} className="space-y-2"><Person id={personId} /><Button type="submit" variant="ghost" size="sm" disabled={clearAffinityPending}>{clearAffinityPending ? "Clearing…" : "Clear rating"}</Button><Status state={clearAffinity} /></form>}
      </TabsContent>
    </Tabs>
  );
}

function Person({ id }: { id: string }) { return <input type="hidden" name="person" value={id} />; }
function Submit({ pending, label }: { pending: boolean; label: string }) { return <Button type="submit" disabled={pending}>{pending ? "Saving…" : label}</Button>; }
function Status({ state }: { state: ActionState }) {
  if (state.status === "idle") return null;
  return <p role="status" className={cn("text-xs", state.status === "success" ? "text-emerald-600 dark:text-emerald-400" : "text-destructive")}>{state.message}</p>;
}
