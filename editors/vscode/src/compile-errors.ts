// Parsing class-library compile errors out of sclang's output stream.
//
// No editor API here, so the format can be tested against real sclang output
// without a running VS Code — which matters, because this format is defined by
// what sclang prints rather than by anything either project controls.
//
// Three lines, verified against sclang 3.13:
//
//     ERROR: syntax error, unexpected '}'
//       in file '/path/to/BadClass.sc'
//       line 4 char 2:
//
// Read a line at a time rather than by a multi-line regex, because the stream
// arrives in chunks that split wherever they like.

const ERROR = /^ERROR: (.+)$/;
const IN_FILE = /^\s+in file '(.+)'$/;
const AT = /^\s+line (\d+) char (\d+):/;

/** sclang prints this when a compile pass begins. */
const COMPILE_START = 'compiling class library';

export interface CompileError {
    file: string;
    /** Zero-based, converted from the one-based number sclang prints. */
    line: number;
    /** Zero-based, likewise. */
    character: number;
    message: string;
}

export interface Batch {
    /** A new compile pass began, so previous errors no longer apply. */
    reset: boolean;
    errors: CompileError[];
}

export class CompileErrorParser {
    /** The tail of the stream, up to the last newline seen. */
    private partial = '';

    private message: string | undefined;
    private file: string | undefined;

    feed(chunk: string): Batch {
        const lines = (this.partial + chunk).split('\n');
        this.partial = lines.pop() ?? '';

        const batch: Batch = { reset: false, errors: [] };
        for (const line of lines) {
            this.line(line, batch);
        }
        return batch;
    }

    private line(line: string, batch: Batch): void {
        // Each pass replaces the last one's errors rather than adding to them,
        // or a file that has since been fixed would stay red until restart.
        if (line.includes(COMPILE_START)) {
            batch.reset = true;
            batch.errors.length = 0;
            this.message = undefined;
            this.file = undefined;
            return;
        }

        const error = ERROR.exec(line);
        if (error) {
            this.message = error[1];
            this.file = undefined;
            return;
        }

        if (this.message === undefined) {
            return;
        }

        const inFile = IN_FILE.exec(line);
        if (inFile) {
            this.file = inFile[1];
            return;
        }

        const at = this.file !== undefined ? AT.exec(line) : null;
        if (at) {
            batch.errors.push({
                file: this.file!,
                line: Number(at[1]) - 1,
                character: Number(at[2]) - 1,
                message: this.message,
            });
        }

        // Matched or not, any other line ends the report. An `ERROR:` with no
        // position — `file '...' parse failed`, which follows every one of
        // these — is a summary, not a second place to look.
        this.message = undefined;
        this.file = undefined;
    }
}
