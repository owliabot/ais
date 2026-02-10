import { describe, it, expect } from 'vitest';
import { encodeFunctionSelector, keccak256 } from '../src/index.js';

describe('EVM utils', () => {
  it('keccak256("") matches known hash', () => {
    expect(keccak256('')).toBe('0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470');
  });

  it('keccak256("hello") matches known hash', () => {
    expect(keccak256('hello')).toBe('0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8');
  });

  it('encodeFunctionSelector(transfer(address,uint256))', () => {
    expect(encodeFunctionSelector('transfer(address,uint256)')).toBe('0xa9059cbb');
  });
});

