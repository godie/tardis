// Minimal static server for the Playwright suite: serves `ui/` so the tests
// load the exact HTML/CSS/JS the Tauri shell loads, without needing a Tauri
// runtime. No dependencies, nothing written to disk.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const port = Number(process.env.PORT ?? 4173);

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");
  const requested = decodeURIComponent(url.pathname);
  const relative = requested === "/" ? "index.html" : requested.replace(/^\/+/, "");
  const filePath = join(root, normalize(relative));

  // Refuse anything that escapes `ui/` via `..`.
  if (!filePath.startsWith(root.endsWith(sep) ? root : root + sep)) {
    response.writeHead(403, { "content-type": "text/plain" }).end("forbidden");
    return;
  }

  try {
    const body = await readFile(filePath);
    response.writeHead(200, {
      "content-type": TYPES[extname(filePath)] ?? "application/octet-stream",
      "cache-control": "no-store",
    });
    response.end(body);
  } catch {
    response.writeHead(404, { "content-type": "text/plain" }).end("not found");
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`ui test server: http://127.0.0.1:${port}`);
});
