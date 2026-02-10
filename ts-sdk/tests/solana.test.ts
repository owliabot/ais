/**
 * Tests for Solana execution compilation (AIS 0.0.2)
 */

import { describe, it, expect } from 'vitest';
import {
  compileSolanaInstruction,
  createDefaultSolanaInstructionCompilerRegistry,
  type SolanaInstructionCompilerRegistry,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  SystemProgram,
  PublicKey,
  TransactionInstruction,
} from '../src/execution/solana/index.js';
import { createContext } from '../src/resolver/context.js';

describe('compileSolanaInstruction', () => {
  it('compiles SPL Token transfer', () => {
    const ctx = createContext();
    ctx.runtime.ctx.wallet_address = 'BPFLoaderUpgradeab1e11111111111111111111111';
    ctx.runtime.calculated.sender_ata = 'DYw8jCTfwHNRJhhmFcbXvVDTqWMEVFBX6ZKUmG5CNSKK';
    ctx.runtime.calculated.recipient_ata = '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
    ctx.runtime.calculated.amount_atomic = '1000000';
    ctx.runtime.contracts.token_program = TOKEN_PROGRAM_ID.toBase58();

    const compiled = compileSolanaInstruction(
      {
        type: 'solana_instruction',
        program: { ref: 'contracts.token_program' },
        instruction: 'transfer',
        discriminator: { lit: '0x03' },
        accounts: [
          { name: 'source', pubkey: { ref: 'calculated.sender_ata' }, signer: { lit: false }, writable: { lit: true } },
          {
            name: 'destination',
            pubkey: { ref: 'calculated.recipient_ata' },
            signer: { lit: false },
            writable: { lit: true },
          },
          { name: 'authority', pubkey: { ref: 'ctx.wallet_address' }, signer: { lit: true }, writable: { lit: false } },
        ],
        data: { object: { amount: { ref: 'calculated.amount_atomic' } } },
        compute_units: { lit: '10000' },
      },
      ctx,
      { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp' }
    );

    expect(compiled.programId.toBase58()).toBe(TOKEN_PROGRAM_ID.toBase58());
    expect(compiled.tx.programId.toBase58()).toBe(TOKEN_PROGRAM_ID.toBase58());
    expect(compiled.tx.keys.length).toBe(3);
    expect(compiled.tx.data[0]).toBe(3); // Transfer
    expect(compiled.computeUnits).toBe(10000);
  });

  it('compiles ATA create_idempotent', () => {
    const ctx = createContext();
    ctx.runtime.ctx.wallet_address = 'BPFLoaderUpgradeab1e11111111111111111111111';
    ctx.runtime.calculated.recipient_ata = '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
    ctx.runtime.params.recipient = 'DYw8jCTfwHNRJhhmFcbXvVDTqWMEVFBX6ZKUmG5CNSKK';
    ctx.runtime.params.mint = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
    ctx.runtime.contracts.ata_program = ASSOCIATED_TOKEN_PROGRAM_ID.toBase58();
    ctx.runtime.contracts.token_program = TOKEN_PROGRAM_ID.toBase58();
    ctx.runtime.contracts.system_program = SystemProgram.programId.toBase58();

    const compiled = compileSolanaInstruction(
      {
        type: 'solana_instruction',
        program: { ref: 'contracts.ata_program' },
        instruction: 'create_idempotent',
        discriminator: { lit: '0x01' },
        accounts: [
          { name: 'payer', pubkey: { ref: 'ctx.wallet_address' }, signer: { lit: true }, writable: { lit: true } },
          { name: 'associated_token', pubkey: { ref: 'calculated.recipient_ata' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'owner', pubkey: { ref: 'params.recipient' }, signer: { lit: false }, writable: { lit: false } },
          { name: 'mint', pubkey: { ref: 'params.mint' }, signer: { lit: false }, writable: { lit: false } },
          { name: 'system_program', pubkey: { ref: 'contracts.system_program' }, signer: { lit: false }, writable: { lit: false } },
          { name: 'token_program', pubkey: { ref: 'contracts.token_program' }, signer: { lit: false }, writable: { lit: false } },
        ],
        data: { object: {} },
      },
      ctx,
      { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp' }
    );

    expect(compiled.programId.toBase58()).toBe(ASSOCIATED_TOKEN_PROGRAM_ID.toBase58());
    expect(compiled.tx.programId.toBase58()).toBe(ASSOCIATED_TOKEN_PROGRAM_ID.toBase58());
    expect(compiled.tx.data[0]).toBe(1); // CreateIdempotent
    expect(compiled.tx.keys.length).toBe(6);
    // includes SystemProgram + token program from SPL helper
    expect(compiled.tx.keys[4]!.pubkey.toBase58()).toBe(SystemProgram.programId.toBase58());
    expect(compiled.tx.keys[5]!.pubkey.toBase58()).toBe(TOKEN_PROGRAM_ID.toBase58());
  });

  it('falls back to generic instruction with bytes data', () => {
    const ctx = createContext();
    const program = new PublicKey('BPFLoaderUpgradeab1e11111111111111111111111').toBase58();
    ctx.runtime.params.program = program;
    ctx.runtime.params.acc = program;

    const compiled = compileSolanaInstruction(
      {
        type: 'solana_instruction',
        program: { ref: 'params.program' },
        instruction: 'custom',
        discriminator: { lit: '0x01' },
        accounts: [{ name: 'a', pubkey: { ref: 'params.acc' }, signer: { lit: false }, writable: { lit: false } }],
        data: { lit: '0x0203' },
      },
      ctx,
      { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp' }
    );

    expect(compiled.tx.data).toEqual(Buffer.from([0x01, 0x02, 0x03]));
  });

  it('uses compiler registry by (programId, instruction)', () => {
    const ctx = createContext();
    const program = new PublicKey('BPFLoaderUpgradeab1e11111111111111111111111');
    ctx.runtime.params.program = program.toBase58();
    ctx.runtime.params.acc = program.toBase58();

    const registry: SolanaInstructionCompilerRegistry = createDefaultSolanaInstructionCompilerRegistry();
    registry.register(program, 'custom', ({ programId, accounts }) => {
      return new TransactionInstruction({
        programId,
        keys: accounts.map((a) => ({ pubkey: a.pubkey, isSigner: a.isSigner, isWritable: a.isWritable })),
        data: Buffer.from([0x09]),
      });
    });

    const compiled = compileSolanaInstruction(
      {
        type: 'solana_instruction',
        program: { ref: 'params.program' },
        instruction: 'custom',
        accounts: [{ name: 'a', pubkey: { ref: 'params.acc' }, signer: { lit: false }, writable: { lit: false } }],
        data: { object: {} },
      },
      ctx,
      { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp', compiler_registry: registry }
    );

    expect(compiled.tx.data).toEqual(Buffer.from([0x09]));
  });
});
