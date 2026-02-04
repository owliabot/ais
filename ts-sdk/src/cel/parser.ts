/**
 * CEL Parser - parse tokens into AST
 */

import { Lexer, Token, TokenType } from './lexer.js';

export type ASTNode =
  | LiteralNode
  | IdentifierNode
  | BinaryNode
  | UnaryNode
  | MemberNode
  | IndexNode
  | CallNode
  | TernaryNode
  | ListNode
  | MapNode;

export interface LiteralNode {
  type: 'Literal';
  value: string | number | boolean | null;
}

export interface IdentifierNode {
  type: 'Identifier';
  name: string;
}

export interface BinaryNode {
  type: 'Binary';
  operator: string;
  left: ASTNode;
  right: ASTNode;
}

export interface UnaryNode {
  type: 'Unary';
  operator: string;
  operand: ASTNode;
}

export interface MemberNode {
  type: 'Member';
  object: ASTNode;
  property: string;
}

export interface IndexNode {
  type: 'Index';
  object: ASTNode;
  index: ASTNode;
}

export interface CallNode {
  type: 'Call';
  callee: ASTNode;
  args: ASTNode[];
}

export interface TernaryNode {
  type: 'Ternary';
  condition: ASTNode;
  consequent: ASTNode;
  alternate: ASTNode;
}

export interface ListNode {
  type: 'List';
  elements: ASTNode[];
}

export interface MapNode {
  type: 'Map';
  entries: Array<{ key: ASTNode; value: ASTNode }>;
}

export class Parser {
  private tokens: Token[] = [];
  private pos = 0;

  parse(input: string): ASTNode {
    const lexer = new Lexer(input);
    this.tokens = lexer.tokenize();
    this.pos = 0;

    const result = this.parseExpression();

    if (this.current().type !== 'EOF') {
      throw new Error(`Unexpected token '${this.current().value}' at position ${this.current().pos}`);
    }

    return result;
  }

  private current(): Token {
    return this.tokens[this.pos] ?? { type: 'EOF', value: null, pos: -1 };
  }

  private advance(): Token {
    const token = this.current();
    this.pos++;
    return token;
  }

  private expect(type: TokenType): Token {
    const token = this.current();
    if (token.type !== type) {
      throw new Error(`Expected ${type} but got ${token.type} at position ${token.pos}`);
    }
    return this.advance();
  }

  private parseExpression(): ASTNode {
    return this.parseTernary();
  }

  private parseTernary(): ASTNode {
    let condition = this.parseOr();

    if (this.current().type === 'QUESTION') {
      this.advance(); // consume ?
      const consequent = this.parseExpression();
      this.expect('COLON');
      const alternate = this.parseExpression();
      condition = {
        type: 'Ternary',
        condition,
        consequent,
        alternate,
      };
    }

    return condition;
  }

  private parseOr(): ASTNode {
    let left = this.parseAnd();

    while (this.current().type === 'OR') {
      const operator = this.advance().value as string;
      const right = this.parseAnd();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseAnd(): ASTNode {
    let left = this.parseEquality();

    while (this.current().type === 'AND') {
      const operator = this.advance().value as string;
      const right = this.parseEquality();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseEquality(): ASTNode {
    let left = this.parseComparison();

    while (this.current().type === 'EQ' || this.current().type === 'NEQ') {
      const operator = this.advance().value as string;
      const right = this.parseComparison();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseComparison(): ASTNode {
    let left = this.parseIn();

    while (
      this.current().type === 'LT' ||
      this.current().type === 'LTE' ||
      this.current().type === 'GT' ||
      this.current().type === 'GTE'
    ) {
      const operator = this.advance().value as string;
      const right = this.parseIn();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseIn(): ASTNode {
    let left = this.parseAdditive();

    if (this.current().type === 'IN') {
      this.advance(); // consume 'in'
      const right = this.parseAdditive();
      left = { type: 'Binary', operator: 'in', left, right };
    }

    return left;
  }

  private parseAdditive(): ASTNode {
    let left = this.parseMultiplicative();

    while (this.current().type === 'PLUS' || this.current().type === 'MINUS') {
      const operator = this.advance().value as string;
      const right = this.parseMultiplicative();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseMultiplicative(): ASTNode {
    let left = this.parseUnary();

    while (
      this.current().type === 'STAR' ||
      this.current().type === 'SLASH' ||
      this.current().type === 'PERCENT'
    ) {
      const operator = this.advance().value as string;
      const right = this.parseUnary();
      left = { type: 'Binary', operator, left, right };
    }

    return left;
  }

  private parseUnary(): ASTNode {
    if (this.current().type === 'NOT' || this.current().type === 'MINUS') {
      const operator = this.advance().value as string;
      const operand = this.parseUnary();
      return { type: 'Unary', operator, operand };
    }

    return this.parsePostfix();
  }

  private parsePostfix(): ASTNode {
    let node = this.parsePrimary();

    while (true) {
      if (this.current().type === 'DOT') {
        this.advance(); // consume .
        const property = this.expect('IDENT').value as string;
        node = { type: 'Member', object: node, property };
      } else if (this.current().type === 'LBRACKET') {
        this.advance(); // consume [
        const index = this.parseExpression();
        this.expect('RBRACKET');
        node = { type: 'Index', object: node, index };
      } else if (this.current().type === 'LPAREN') {
        this.advance(); // consume (
        const args: ASTNode[] = [];
        if (this.current().type !== 'RPAREN') {
          args.push(this.parseExpression());
          while (this.current().type === 'COMMA') {
            this.advance(); // consume ,
            args.push(this.parseExpression());
          }
        }
        this.expect('RPAREN');
        node = { type: 'Call', callee: node, args };
      } else {
        break;
      }
    }

    return node;
  }

  private parsePrimary(): ASTNode {
    const token = this.current();

    switch (token.type) {
      case 'NUMBER':
      case 'STRING':
      case 'BOOL':
      case 'NULL':
        this.advance();
        return { type: 'Literal', value: token.value };

      case 'IDENT':
        this.advance();
        return { type: 'Identifier', name: token.value as string };

      case 'LPAREN': {
        this.advance(); // consume (
        const expr = this.parseExpression();
        this.expect('RPAREN');
        return expr;
      }

      case 'LBRACKET': {
        this.advance(); // consume [
        const elements: ASTNode[] = [];
        if (this.current().type !== 'RBRACKET') {
          elements.push(this.parseExpression());
          while (this.current().type === 'COMMA') {
            this.advance(); // consume ,
            elements.push(this.parseExpression());
          }
        }
        this.expect('RBRACKET');
        return { type: 'List', elements };
      }

      default:
        throw new Error(`Unexpected token '${token.type}' at position ${token.pos}`);
    }
  }
}
