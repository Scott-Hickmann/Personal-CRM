import { Skeleton } from "@/components/ui/skeleton";

export default function Loading() {
  return <div className="space-y-5 p-8"><Skeleton className="h-10 w-56" /><Skeleton className="h-24 w-full" /><div className="grid gap-4 lg:grid-cols-2"><Skeleton className="h-56" /><Skeleton className="h-56" /></div></div>;
}
