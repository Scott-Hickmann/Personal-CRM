import { runCrm } from "@/lib/crm";

type ContactImage = { version: string; mime_type: string; data: string };

export async function GET(request: Request, { params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const image = await runCrm<ContactImage | null>(["ui-data", "image", id]);
  if (!image) return new Response(null, { status: 404, headers: { "Cache-Control": "no-store" } });
  const headers = {
    "Content-Type": image.mime_type,
    "Cache-Control": "private, no-cache",
    "ETag": `"${image.version}"`,
    "X-Content-Type-Options": "nosniff",
  };
  if (request.headers.get("if-none-match") === headers.ETag) {
    return new Response(null, { status: 304, headers });
  }
  return new Response(Buffer.from(image.data, "base64"), { headers });
}
