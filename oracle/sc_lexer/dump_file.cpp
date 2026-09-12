// Reads a FILE and dumps its token stream, using SuperCollider's own sc_lexer.
//
// Upstream's standalone driver takes source as argv[1] and measures it with
// strlen, so it cannot handle a class-library file (ARG_MAX) or one containing
// a NUL. Only the I/O differs here; the lexing is entirely theirs, which is
// what makes this usable as an oracle.
//
// Output: one token per line, `<TypeName> <byte_length>`. Kinds plus lengths
// pin the tiling exactly, with no escaping to get wrong.

#include "codepoint_stream.hpp"
#include "normalise_source.hpp"
#include "text_location.hpp"
#include <fstream>
#include <iostream>
#include <lexer.hpp>
#include <sstream>

using namespace sc::lex;

int main(int argc, char* argv[]) {
    if (argc != 2) {
        std::cerr << "usage: dump_file <path>\n";
        return 2;
    }
    std::ifstream in(argv[1], std::ios::binary);
    if (!in) {
        std::cerr << "cannot open " << argv[1] << "\n";
        return 1;
    }
    std::stringstream buf;
    buf << in.rdbuf();
    const std::string source = buf.str();

    NormalisedSource src { source.c_str(), source.size() };
    CodePointStream stream { std::move(src), {} };
    actions::TypeAndLocationAction action {};

    for (auto r = lexer(stream, action); r.type != TokenType::EndOfFile; r = lexer(stream, action)) {
        const auto [ptr, sz] = stream.source_code_range_to_text(r.range);
        (void)ptr;
        std::cout << to_string(r.type) << ' ' << sz << '\n';
    }
    return 0;
}
