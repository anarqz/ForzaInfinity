// Decompile a list of addresses into <outdir>/<addr>.c, creating functions as needed.
// Usage (headless): -postScript Decomp.java <outdir> <addr> [<addr> ...]
// An argument "@file" reads addresses (one hex per line) from that file.
//@category FH2Online
import ghidra.app.script.GhidraScript;
import ghidra.app.cmd.function.CreateFunctionCmd;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public class Decomp extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        File out = new File(args[0]);
        out.mkdirs();
        List<String> addrs = new ArrayList<>();
        for (int i = 1; i < args.length; i++) {
            if (args[i].startsWith("@")) {
                for (String l : Files.readAllLines(Paths.get(args[i].substring(1)))) {
                    l = l.trim();
                    if (!l.isEmpty()) addrs.add(l);
                }
            } else addrs.add(args[i]);
        }
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        for (String s : addrs) {
            Address a = toAddr(Long.parseUnsignedLong(s.replace("0x", ""), 16));
            Function f = getFunctionContaining(a);
            if (f == null) {
                new DisassembleCommand(a, null, true).applyTo(currentProgram, monitor);
                new CreateFunctionCmd(a).applyTo(currentProgram, monitor);
                f = getFunctionAt(a);
            }
            String text;
            if (f == null) text = "// could not create function at " + a + "\n";
            else {
                DecompileResults r = di.decompileFunction(f, 120, monitor);
                text = r.decompileCompleted() ? r.getDecompiledFunction().getC()
                        : "// decompile failed: " + r.getErrorMessage() + "\n";
                text = "// " + f.getName() + " @ " + f.getEntryPoint() + "\n" + text;
            }
            Files.write(new File(out, s.replace("0x", "") + ".c").toPath(), text.getBytes("UTF-8"));
            println("decompiled " + s);
        }
        di.dispose();
    }
}
