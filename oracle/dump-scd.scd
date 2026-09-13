// Ask sclang whether each snippet of the script corpus parses.
//
// `String:compile` is `thisProcess.interpreter.compile`, which compiles a
// string as `cmdlinecode` and hands back a Function — or nil if it did not
// parse. It runs nothing, which is the whole reason this oracle is safe: the
// corpus is thousands of real examples that would boot servers, open windows
// and make noise if any of them were evaluated.
//
// It is also exactly the right production. A `.scd` file is `cmdlinecode`, not
// a class file, so this checks the syntax scratch buffers are written in —
// which the class library never exercises.
//
// Output is tab-separated:
//
//   S <snippet> ok|fail
//
//   SCLANG_SCD_CORPUS=<dir> sclang -i none dump-scd.scd
//
// Writes to the path given by SCLANG_ORACLE_OUT, or ./oracle-scd.tsv.

(
var dir, out, path, names, ok = 0, failed = 0;

dir = "SCLANG_SCD_CORPUS".getenv ?? { "scd-corpus" };
path = "SCLANG_ORACLE_OUT".getenv ?? { "oracle-scd.tsv" };

out = File(path, "w");
if (out.isOpen.not) {
    ("could not open" + path).postln;
    0.exit;
};

names = (dir +/+ "*.scd").pathMatch.sort;

names.do { |file|
    var code, name, compiled;
    name = PathName(file).fileName;
    code = File.readAllString(file);

    // A snippet that fails to compile posts its error, which would bury the
    // result in tens of thousands of lines. The errors are not what this
    // compares — whether it compiled at all is.
    compiled = try { code.compile.notNil } { false };

    out.write("S\t%\t%\n".format(name, if (compiled) { "ok" } { "fail" }));
    if (compiled) { ok = ok + 1 } { failed = failed + 1 };
};

out.close;
("oracle: % snippets, % compiled, % did not -> %".format(ok + failed, ok, failed, path)).postln;
0.exit;
)
