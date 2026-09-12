// Dump the compiled class library as a symbol table, for differential testing
// against sclang-syntax.
//
// This is the *reachable* oracle. `DumpParseNode.cpp` would give a full parse
// tree, but nothing in sclang ever calls `dump()` — it has no flag, no
// primitive and no caller — so reaching it would mean patching and rebuilding
// SuperCollider. Runtime reflection needs none of that, and it validates
// exactly what a language server's index has to get right: which classes and
// methods exist, where they are defined, and what arguments they take.
//
// Output is tab-separated so it cannot be broken by quoting:
//
//   C <name> <superclass> <file> <charPos>
//   M <class> <method> i|c <file> <charPos> <arg,arg,...>
//
//   sclang -i none dump-symbols.scd
//
// Writes to the path given by SCLANG_ORACLE_OUT, or ./oracle-symbols.tsv.

(
var out, path, classCount = 0, methodCount = 0;

path = "SCLANG_ORACLE_OUT".getenv ?? { "oracle-symbols.tsv" };
out = File(path, "w");

if (out.isOpen.not) {
    ("could not open" + path).postln;
    0.exit;
};

Class.allClasses.do { |class|
    // Metaclasses are reached through their instance class, not on their own.
    if (class.isMetaClass.not) {
        out.write(
            "C\t%\t%\t%\t%\n".format(
                class.name,
                class.superclass !? { |s| s.name } ?? { "-" },
                class.filenameSymbol,
                class.charPos
            )
        );
        classCount = classCount + 1;

        [[class, $i], [class.class, $c]].do { |pair|
            var target = pair[0], kind = pair[1];
            (target.methods ?? []).do { |m|
                out.write(
                    "M\t%\t%\t%\t%\t%\t%\n".format(
                        class.name,
                        m.name,
                        kind,
                        m.filenameSymbol,
                        m.charPos,
                        (m.argNames ?? []).join(",")
                    )
                );
                methodCount = methodCount + 1;
            };
        };
    };
};

out.close;
("oracle: % classes, % methods -> %".format(classCount, methodCount, path)).postln;
0.exit;
)
