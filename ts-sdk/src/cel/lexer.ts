/**
 * CEL Lexer - tokenize CEL expressions
 */

export type TokenType =
  | 'NUMBER'
  | 'STRING'
  | 'BOOL'
  | 'NULL'
  | 'IDENT'
  | 'DOT'
  | 'COMMA'
  | 'LPAREN'
  | 'RPAREN'
  | 'LBRACKET'
  | 'RBRACKET'
  | 'PLUS'
  | 'MINUS'
  | 'STAR'
  | 'SLASH'
  | 'PERCENT'
  | 'EQ'
  | 'NEQ'
  | 'LT'
  | 'LTE'
  | 'GT'
  | 'GTE'
  | 'AND'
  | 'OR'
  | 'NOT'
  | 'QUESTION'
  | 'COLON'
  | 'IN'
  | 'EOF';

export interface Token {
  type: TokenType;
  value: string | boolean | null;
  pos: number;
}

const KEYWORDS: Record<string, TokenType> = {
  true: 'BOOL',
  false: 'BOOL',
  null: 'NULL',
  in: 'IN',
};

export class Lexer {
  private pos = 0;
  private input: string;

  constructor(input: string) {
    this.input = input;
  }

  tokenize(): Token[] {
    const tokens: Token[] = [];

    while (this.pos < this.input.length) {
      const char = this.input[this.pos];

      // Skip whitespace
      if (/\s/.test(char)) {
        this.pos++;
        continue;
      }

      // Numbers
      if (/\d/.test(char) || (char === '.' && /\d/.test(this.peek(1)))) {
        tokens.push(this.readNumber());
        continue;
      }

      // Strings
      if (char === '"' || char === "'") {
        tokens.push(this.readString(char));
        continue;
      }

      // Identifiers and keywords
      if (/[a-zA-Z_]/.test(char)) {
        tokens.push(this.readIdentifier());
        continue;
      }

      // Two-character operators
      const twoChar = this.input.slice(this.pos, this.pos + 2);
      if (twoChar === '==') {
        tokens.push({ type: 'EQ', value: '==', pos: this.pos });
        this.pos += 2;
        continue;
      }
      if (twoChar === '!=') {
        tokens.push({ type: 'NEQ', value: '!=', pos: this.pos });
        this.pos += 2;
        continue;
      }
      if (twoChar === '<=') {
        tokens.push({ type: 'LTE', value: '<=', pos: this.pos });
        this.pos += 2;
        continue;
      }
      if (twoChar === '>=') {
        tokens.push({ type: 'GTE', value: '>=', pos: this.pos });
        this.pos += 2;
        continue;
      }
      if (twoChar === '&&') {
        tokens.push({ type: 'AND', value: '&&', pos: this.pos });
        this.pos += 2;
        continue;
      }
      if (twoChar === '||') {
        tokens.push({ type: 'OR', value: '||', pos: this.pos });
        this.pos += 2;
        continue;
      }

      // Single-character operators
      const singleCharTokens: Record<string, TokenType> = {
        '.': 'DOT',
        ',': 'COMMA',
        '(': 'LPAREN',
        ')': 'RPAREN',
        '[': 'LBRACKET',
        ']': 'RBRACKET',
        '+': 'PLUS',
        '-': 'MINUS',
        '*': 'STAR',
        '/': 'SLASH',
        '%': 'PERCENT',
        '<': 'LT',
        '>': 'GT',
        '!': 'NOT',
        '?': 'QUESTION',
        ':': 'COLON',
      };

      if (singleCharTokens[char]) {
        tokens.push({ type: singleCharTokens[char], value: char, pos: this.pos });
        this.pos++;
        continue;
      }

      throw new Error(`Unexpected character '${char}' at position ${this.pos}`);
    }

    tokens.push({ type: 'EOF', value: null, pos: this.pos });
    return tokens;
  }

  private peek(offset: number): string {
    return this.input[this.pos + offset] ?? '';
  }

  private readNumber(): Token {
    const start = this.pos;
    let hasDecimal = false;

    while (this.pos < this.input.length) {
      const char = this.input[this.pos];
      if (/\d/.test(char)) {
        this.pos++;
      } else if (char === '.' && !hasDecimal) {
        hasDecimal = true;
        this.pos++;
      } else {
        break;
      }
    }

    const value = this.input.slice(start, this.pos);
    return {
      type: 'NUMBER',
      value,
      pos: start,
    };
  }

  private readString(quote: string): Token {
    const start = this.pos;
    this.pos++; // Skip opening quote

    let value = '';
    while (this.pos < this.input.length) {
      const char = this.input[this.pos];
      if (char === quote) {
        this.pos++; // Skip closing quote
        return { type: 'STRING', value, pos: start };
      }
      if (char === '\\') {
        this.pos++;
        const escaped = this.input[this.pos];
        const escapeMap: Record<string, string> = {
          n: '\n',
          t: '\t',
          r: '\r',
          '\\': '\\',
          '"': '"',
          "'": "'",
        };
        value += escapeMap[escaped] ?? escaped;
      } else {
        value += char;
      }
      this.pos++;
    }

    throw new Error(`Unterminated string starting at position ${start}`);
  }

  private readIdentifier(): Token {
    const start = this.pos;

    while (this.pos < this.input.length && /[a-zA-Z0-9_]/.test(this.input[this.pos])) {
      this.pos++;
    }

    const value = this.input.slice(start, this.pos);
    const keywordType = KEYWORDS[value];

    if (keywordType === 'BOOL') {
      return { type: 'BOOL', value: value === 'true', pos: start };
    }
    if (keywordType === 'NULL') {
      return { type: 'NULL', value: null, pos: start };
    }
    if (keywordType) {
      return { type: keywordType, value, pos: start };
    }

    return { type: 'IDENT', value, pos: start };
  }
}
