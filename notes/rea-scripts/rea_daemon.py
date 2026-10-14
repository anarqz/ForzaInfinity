# Holds one REA MCP session with azimuth.exe open in headless Ghidra, and
# serves tool calls over a Unix socket so each query reuses the same import.
import asyncio, json, os, sys, datetime
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SOCK = "/home/claude/work/rea.sock"
TARGET = sys.argv[1]
TIMEOUT = 3600.0

def text_of(res):
    parts = []
    for c in res.content:
        parts.append(getattr(c, "text", None) or str(c))
    return "\n".join(parts)

async def main():
    env = dict(os.environ,
               GHIDRA_INSTALL_DIR="/home/claude/tools/ghidra_12.1.4_PUBLIC",
               REA_GHIDRA_STARTUP_TIMEOUT_MS="3600000")
    p = StdioServerParameters(command="rea", args=["--mcp"], env=env)
    async with stdio_client(p) as (r, w):
        async with ClientSession(r, w) as s:
            await s.initialize()
            print("opening", TARGET, flush=True)
            res = await s.call_tool("open_binary", {"path": TARGET, "provider_id": "ghidra"},
                                    read_timeout_seconds=TIMEOUT)
            print("open_binary ->", text_of(res)[:2000], flush=True)
            lock = asyncio.Lock()

            async def handle(reader, writer):
                try:
                    req = json.loads((await reader.readline()).decode())
                    async with lock:
                        res = await s.call_tool(req["tool"], req.get("args", {}),
                                                read_timeout_seconds=TIMEOUT)
                    out = {"isError": bool(res.is_error), "text": text_of(res)}
                except Exception as e:
                    out = {"isError": True, "text": f"{type(e).__name__}: {e}"}
                writer.write((json.dumps(out) + "\n").encode())
                await writer.drain(); writer.close()

            if os.path.exists(SOCK): os.unlink(SOCK)
            server = await asyncio.start_unix_server(handle, path=SOCK, limit=2**30)
            print("READY", flush=True)
            async with server:
                await server.serve_forever()

asyncio.run(main())
