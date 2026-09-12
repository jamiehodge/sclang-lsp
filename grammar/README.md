# Derived grammar

`sclang.y` is SuperCollider's own bison grammar (`lang/LangSource/Bison/lang11d`)
with the semantic actions stripped. It is the input for the parser, and it is
regenerated rather than edited by hand.

    bison -v -o sclang.tab.c sclang.y   # 0 conflicts

Error recovery productions will be *added* here. sclang has none — its parser
aborts on the first syntax error — which is the one thing a language server
cannot live with.
