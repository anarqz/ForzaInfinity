import asyncio, os, sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
async def main():
    env = dict(os.environ, GHIDRA_INSTALL_DIR="/home/claude/tools/ghidra_12.1.4_PUBLIC")
    p = StdioServerParameters(command="rea", args=["--mcp"], env=env)
    async with stdio_client(p) as (r, w):
        async with ClientSession(r, w) as s:
            await s.initialize()
            tools = (await s.list_tools()).tools
            print(len(tools), "tools")
            for t in tools:
                d = (t.description or "").split("\n")[0][:110]
                print(f"{t.name:34} {d}")
asyncio.run(main())
