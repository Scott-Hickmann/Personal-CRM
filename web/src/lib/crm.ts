import "server-only";

import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";
import { connection } from "next/server";
import type { InteractionBody, Overview, PersonDetail } from "@/lib/types";

const execute = promisify(execFile);

type Envelope<T> = { schema_version: string; command: string; data: T };

function executablePath() {
  return process.env.CRM_CLI_PATH ?? path.resolve(process.cwd(), "..", "target", "debug", "crm");
}

export async function runCrm<T>(args: string[]): Promise<T> {
  try {
    const cliArgs = ["--format", "json"];
    if (process.env.CRM_CONFIG_PATH) cliArgs.push("--config", process.env.CRM_CONFIG_PATH);
    cliArgs.push(...args);
    const { stdout } = await execute(executablePath(), cliArgs, {
      maxBuffer: 25 * 1024 * 1024,
    });
    return (JSON.parse(stdout) as Envelope<T>).data;
  } catch (error) {
    const stderr = error && typeof error === "object" && "stderr" in error ? String(error.stderr) : "";
    let message = error instanceof Error ? error.message : "CRM command failed";
    try {
      message = JSON.parse(stderr).error?.message ?? message;
    } catch {
      // The process error still contains the executable and exit status.
    }
    throw new Error(message);
  }
}

export async function getOverview(): Promise<Overview> {
  await connection();
  return runCrm<Overview>(["ui-data", "overview"]);
}

export async function getPerson(id: string): Promise<PersonDetail> {
  await connection();
  return runCrm<PersonDetail>(["ui-data", "person", id, "--history-limit", "200"]);
}

export function getInteraction(id: string): Promise<InteractionBody> {
  return runCrm<InteractionBody>(["ui-data", "interaction", id]);
}
