// Somewhere to try the language features. Open this in the Extension
// Development Host and use it from the top down.

// Hover: the class, then the method. The class shows its superclass chain,
// the method its signature and the comment above its definition.
SinOsc.ar(440, 0, 0.2);

// Goto definition: on SinOsc jumps to Osc.sc; on `multiNew` it resolves
// through the superclass chain to UGen, because dispatch from a literal class
// name is the one case that is statically known.
SinOsc.multiNew(\audio, 440);

// Completion: type a dot after this class and the list is class-side methods,
// inherited ones included.
Pbind

// With no type for the receiver there is no way to know which class this is,
// so completion offers every selector that exists. Type a dot after `x`.
x

// Diagnostics: this line is incomplete, and the error should appear as you
// type rather than on save. Delete the closing brace to see it move.
Foo { bar { ^1 } }

// Document symbols (⇧⌘O) lists the class above with its method nested inside.
