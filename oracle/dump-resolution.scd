// Dump what method dispatch actually selects, for differential testing against
// the server's receiver narrowing.
//
// `Foo(...).bar` used to resolve to every class defining `bar`. It now resolves
// to the one an instance of Foo would really call, by walking Foo's superclass
// chain — and this is how that walk is checked, against the only authority on
// the matter.
//
// `findRespondingMethodFor` is sclang's own dispatch resolution: it is what
// `Object` itself uses, it needs no instance, and it has no side effects. That
// last part is why this oracle is possible at all — actually constructing one
// of every class would boot servers, open windows and throw.
//
// For each class it emits every selector anywhere in that class's own
// superclass chain. Overrides are where a walk can pick the *wrong*
// implementation, and are counted separately below; the rest still check that
// it finds the right one at all, which is the half that actually changed.
//
// Output is tab-separated:
//
//   R <class> <selector> <owning class>
//
//   sclang -i none dump-resolution.scd
//
// Writes to the path given by SCLANG_ORACLE_OUT, or ./oracle-resolution.tsv.

(
var out, path, pairs = 0, classes = 0, overrides = 0;

path = "SCLANG_ORACLE_OUT".getenv ?? { "oracle-resolution.tsv" };
out = File(path, "w");

if (out.isOpen.not) {
    ("could not open" + path).postln;
    0.exit;
};

Class.allClasses.do { |class|
    var chain, counts, selectors, walker;

    // Metaclasses are reached through their instance class, and instance-side
    // dispatch is what the narrowing is about.
    if (class.isMetaClass.not) {
        chain = List.new;
        walker = class;
        while { walker.notNil } { chain.add(walker); walker = walker.superclass };

        // How many classes in this chain define each selector.
        counts = IdentityDictionary.new;
        chain.do { |k|
            (k.methods ?? []).do { |m| counts[m.name] = (counts[m.name] ? 0) + 1 };
        };

        selectors = counts.keys;

        selectors.do { |sel|
            var m = class.findRespondingMethodFor(sel);
            if (m.notNil) {
                out.write("R\t%\t%\t%\n".format(class.name, sel, m.ownerClass.name));
                pairs = pairs + 1;
                if (counts[sel] > 1) { overrides = overrides + 1 };
            };
        };

        classes = classes + 1;
    };
};

out.close;
("oracle: % classes, % pairs (% of them overrides) -> %"
    .format(classes, pairs, overrides, path)).postln;
0.exit;
)
