"use server";

import { runCrm } from "@/lib/crm";
import type { ActionState } from "@/lib/types";

export async function setClusterName(id: string, name: string | null): Promise<ActionState> {
  if (!/^cluster-[a-f0-9]{20}$/.test(id) || (name !== null && (!name.trim() || [...name.trim()].length > 80 || /[\x00-\x1f\x7f]/.test(name)))) {
    return { status: "error", message: "Enter a cluster name between 1 and 80 characters." };
  }
  try {
    await runCrm(name === null ? ["cluster", "reset-name", id] : ["cluster", "rename", id, "--", name.trim()]);
    return { status: "success", message: name === null ? "Suggested name restored." : "Cluster name saved." };
  } catch (error) {
    return { status: "error", message: error instanceof Error ? error.message : "Unable to save cluster name." };
  }
}
