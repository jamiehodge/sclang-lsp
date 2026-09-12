// sclang-lsp — a guided tour.
//
// Work down the file with the extension active. Everything below is answered
// by parsing SuperCollider directly: no sclang is running, and none of it
// would stop working if the class library failed to compile.

// ── 1. Hover ────────────────────────────────────────────────────────────
// Hover `SinOsc` for its superclass chain, then `ar` for its signature and
// the comment above its definition.
SinOsc.ar(440, 0, 0.2);

// ── 2. Goto definition — F12 ────────────────────────────────────────────
// On `SinOsc`, jumps to Osc.sc in the class library. On `multiNew`, resolves
// through the superclass chain to UGen: dispatch from a literal class name is
// the one case that is statically known.
SinOsc.multiNew(\audio, 440);

// Here the receiver is a variable, so nothing says which class this is. Every
// class defining `play` is offered at once, which is the honest answer rather
// than a guess.
x.play;

// ── 3. Signature help ───────────────────────────────────────────────────
// Put the cursor between the parens. The parameter you are on is highlighted;
// a comma advances it, and typing `mul:` jumps to that parameter wherever it
// sits in the list.
SinOsc.ar();

// ── 4. Inlay hints ──────────────────────────────────────────────────────
// The parameter each positional argument fills is shown inline.
SinOsc.ar(440, 0, 0.5);

// Nothing is labelled here, deliberately. `x` could be anything and `ar` is
// defined on many classes with different parameter names — and an inlay hint
// renders as though it were in the source, so a guess would read as a fact.
x.ar(440, 0);

// ── 5. Completion ───────────────────────────────────────────────────────
// On the blank line below, type `SinOsc.` for class-side methods with the
// inherited ones included, then `SinOsc.ar(` for the parameter names.
//
// Leave the line blank rather than parking a bare `SinOsc` here: a class name
// followed by a brace is a class *definition*, so it would swallow the block
// in the next section.


// ── 6. Names you wrote yourself ─────────────────────────────────────────
{
	var carrier = 440, index = 3;
	var modulator = carrier * index;

	// Completion here offers `carrier`, `index` and `modulator` above any
	// global that merely shares a prefix. Hover and goto-definition work on
	// them, and find-references — ⇧F12 — stays inside this function.
	modulator.postln;
};

// Shadowing resolves the way the language resolves it: goto-definition on the
// inner `v` lands on the inner binding, not the outer one.
{ |v|
	{ |v| v.postln };
};

// ── 7. Find references — ⇧F12 ───────────────────────────────────────────
// On `SinOsc`: every mention across the class library, exactly, because only
// class names lex as class names — a \SinOsc symbol or a "SinOsc" string is
// left out.
//
// On `.postln`: a couple of hundred hits across the workspace. That one is
// textual and says so. Dispatch is dynamic, so every `.postln` is a candidate
// for any class's `postln` and there is no way to narrow it without types.
SinOsc.ar(440).postln;

// ── 8. Rename — F2 ──────────────────────────────────────────────────────
// Works on the locals in section 6, and on a class name.
//
// Refused on a method, with the reason given before the rename box opens.
// Renaming every `.ar` in a workspace would break the classes that were not
// meant, and renaming only some would break this one. Instance variables are
// refused too: `var <count` generates the methods `count` and `count_`, and
// subclasses inherit both.
//
// Try F2 on `ar` below to see the refusal.
SinOsc.ar(440);

// ── 9. Document symbols — ⇧⌘O ───────────────────────────────────────────
// Classes with their methods nested underneath. Try it on any .sc file in the
// class library.

// ── 10. Diagnostics ─────────────────────────────────────────────────────
// Reported as you type rather than on save, and on a buffer that has never
// been written to disk. The call below is missing its closing paren; add one
// and the squiggle goes.
SinOsc.ar(440
