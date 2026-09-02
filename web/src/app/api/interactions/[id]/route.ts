import { getInteraction } from "@/lib/crm";

export async function GET(_: Request, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  try {
    const interaction = await getInteraction(id);
    return Response.json(interaction, { headers: { "Cache-Control": "no-store" } });
  } catch (error) {
    return Response.json(
      { error: error instanceof Error ? error.message : "Interaction not found" },
      { status: 404, headers: { "Cache-Control": "no-store" } },
    );
  }
}
