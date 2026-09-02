"use server";

import { revalidatePath } from "next/cache";
import { runCrm } from "@/lib/crm";
import type { ActionState } from "@/lib/types";

const empty = (value: FormDataEntryValue | null) => String(value ?? "").trim();

async function mutate(args: string[], personId: string, success: string): Promise<ActionState> {
  try {
    await runCrm(args);
    revalidatePath("/");
    revalidatePath(`/people/${encodeURIComponent(personId)}`);
    return { status: "success", message: success };
  } catch (error) {
    return { status: "error", message: error instanceof Error ? error.message : "CRM update failed" };
  }
}

export async function addNoteAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  const text = empty(formData.get("text"));
  if (!person || !text) return { status: "error", message: "A note is required." };
  return mutate(["note", "add", "--person", person, "--text", text], person, "Note added.");
}

export async function setFactAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  const key = empty(formData.get("key"));
  const value = empty(formData.get("value"));
  if (!person || !key || !value) return { status: "error", message: "A key and value are required." };
  return mutate(["fact", "set", "--person", person, "--key", key, "--value", value], person, "Fact saved.");
}

export async function addTagAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  const tag = empty(formData.get("tag")).replace(/^#/, "");
  if (!person || !tag) return { status: "error", message: "A tag is required." };
  return mutate(["tag", "add", "--person", person, "--tag", tag], person, "Tag added.");
}

export async function addFollowupAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  const text = empty(formData.get("text"));
  const due = empty(formData.get("due"));
  if (!person || !text) return { status: "error", message: "A follow-up is required." };
  const args = ["followup", "add", "--person", person, "--text", text];
  if (due) args.push("--due", due);
  return mutate(args, person, "Follow-up added.");
}

export async function setAffinityAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  const rating = empty(formData.get("rating"));
  const value = Number(rating);
  if (!person || !Number.isInteger(value) || value < 1 || value > 7) {
    return { status: "error", message: "Choose a closeness rating from 1 to 7." };
  }
  return mutate(["affinity", "rate", "--person", person, "--rating", rating], person, `Closeness rated ${rating}/7.`);
}

export async function clearAffinityAction(_: ActionState, formData: FormData): Promise<ActionState> {
  const person = empty(formData.get("person"));
  if (!person) return { status: "error", message: "A person is required." };
  return mutate(["affinity", "clear", "--person", person], person, "Closeness rating cleared.");
}
