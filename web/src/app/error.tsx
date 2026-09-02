"use client";

import { AlertTriangle } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

export default function ErrorPage({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return <div className="mx-auto max-w-2xl p-8"><Alert variant="destructive"><AlertTriangle /><AlertTitle>Could not load the CRM</AlertTitle><AlertDescription className="space-y-4"><p>{error.message}</p><Button variant="outline" onClick={reset}>Try again</Button></AlertDescription></Alert></div>;
}
