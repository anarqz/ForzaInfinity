import asyncio, os, sys, json
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
async def main():
    env = dict(os.environ, GHIDRA_INSTALL_DIR="/home/claude/tools/ghidra_12.1.4_PUBLIC")
    p = StdioServerParameters(command="rea", args=["--mcp"], env=env)
    async with stdio_client(p) as (r, w):
        async with ClientSession(r, w) as s:
            await s.initialize()
            for t in (await s.list_tools()).tools:
                if t.name in sys.argv[1:]:
                    print("==", t.name); print(json.dumps(t.input_schema, indent=1)[:2500])
asyncio.run(main())
