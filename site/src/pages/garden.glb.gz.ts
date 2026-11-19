import { gzipSync } from 'node:zlib';
import model from '../../../jianghu/assets/mew-diorama.glb?url&inline';

export async function GET() {
  const original = await fetch(model);
  const compressed = gzipSync(new Uint8Array(await original.arrayBuffer()), { level: 9 });
  return new Response(compressed, { headers: { 'Content-Type': 'application/gzip' } });
}
