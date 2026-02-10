import { describe, it, expect } from 'vitest';
import {
  buildFunctionSignatureFromJsonAbi,
  encodeFunctionSelector,
  encodeJsonAbiFunctionCall,
  decodeJsonAbiFunctionResult,
  AbiArgsError,
  AbiDecodingError,
} from '../src/index.js';

function pad32(hexNo0x: string): string {
  return hexNo0x.replace(/^0x/, '').padStart(64, '0');
}

describe('JSON ABI encoding (AIS 0.0.2)', () => {
  it('builds canonical signature for tuple inputs', () => {
    const abi = {
      type: 'function' as const,
      name: 'exactInputSingle',
      inputs: [
        {
          name: 'params',
          type: 'tuple',
          components: [
            { name: 'tokenIn', type: 'address' },
            { name: 'tokenOut', type: 'address' },
            { name: 'fee', type: 'uint24' },
            { name: 'recipient', type: 'address' },
            { name: 'deadline', type: 'uint256' },
            { name: 'amountIn', type: 'uint256' },
            { name: 'amountOutMinimum', type: 'uint256' },
            { name: 'sqrtPriceLimitX96', type: 'uint160' },
          ],
        },
      ],
      outputs: [],
    };

    expect(buildFunctionSignatureFromJsonAbi(abi)).toBe(
      'exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))'
    );
  });

  it('encodes tuple args by name and preserves component order', () => {
    const abi = {
      type: 'function' as const,
      name: 'exactInputSingle',
      inputs: [
        {
          name: 'params',
          type: 'tuple',
          components: [
            { name: 'tokenIn', type: 'address' },
            { name: 'tokenOut', type: 'address' },
            { name: 'fee', type: 'uint24' },
            { name: 'recipient', type: 'address' },
            { name: 'deadline', type: 'uint256' },
            { name: 'amountIn', type: 'uint256' },
            { name: 'amountOutMinimum', type: 'uint256' },
            { name: 'sqrtPriceLimitX96', type: 'uint160' },
          ],
        },
      ],
      outputs: [],
    };

    const signature = buildFunctionSignatureFromJsonAbi(abi);
    const selector = encodeFunctionSelector(signature);

    const tokenIn = '0x1111111111111111111111111111111111111111';
    const tokenOut = '0x2222222222222222222222222222222222222222';
    const recipient = '0x3333333333333333333333333333333333333333';

    const data = encodeJsonAbiFunctionCall(abi, {
      params: {
        tokenIn,
        tokenOut,
        fee: 3000n,
        recipient,
        deadline: 1n,
        amountIn: 2n,
        amountOutMinimum: 3n,
        sqrtPriceLimitX96: 0n,
      },
    });

    expect(data.startsWith(selector)).toBe(true);
    expect(data.length).toBe(10 + 8 * 64);

    const expected =
      selector +
      pad32(tokenIn) +
      pad32(tokenOut) +
      pad32('0x' + 3000n.toString(16)) +
      pad32(recipient) +
      pad32('0x1') +
      pad32('0x2') +
      pad32('0x3') +
      pad32('0x0');

    expect(data).toBe(expected);
  });

  it('validates missing/extra ABI args', () => {
    const abi = {
      type: 'function' as const,
      name: 'f',
      inputs: [
        { name: 'a', type: 'uint256' },
        { name: 'b', type: 'address' },
      ],
      outputs: [],
    };

    expect(() => encodeJsonAbiFunctionCall(abi, { a: 1n } as any)).toThrow(AbiArgsError);
    expect(() => encodeJsonAbiFunctionCall(abi, { a: 1n, b: '0x' + '11'.repeat(20), c: 0 } as any)).toThrow(
      AbiArgsError
    );
  });

  it('encodes dynamic string with correct offset', () => {
    const abi = {
      type: 'function' as const,
      name: 'g',
      inputs: [
        { name: 's', type: 'string' },
        { name: 'x', type: 'uint256' },
      ],
      outputs: [],
    };

    const signature = buildFunctionSignatureFromJsonAbi(abi);
    const selector = encodeFunctionSelector(signature);

    const data = encodeJsonAbiFunctionCall(abi, { s: 'hi', x: 5n });
    expect(data.startsWith(selector)).toBe(true);

    // head size = 32 (offset) + 32 (uint256) = 64 bytes -> string offset 0x40
    const head =
      pad32('0x40') + // offset to string tail
      pad32('0x5');

    // tail = len(2) + "hi" bytes padded
    const tail = pad32('0x2') + '6869'.padEnd(64, '0');

    expect(data).toBe(selector + head + tail);
  });
});

describe('JSON ABI decoding (AIS 0.0.2)', () => {
  function encodeReturnData(outputsAsInputs: any, args: Record<string, unknown>): string {
    const abi = {
      type: 'function' as const,
      name: 'ret',
      inputs: outputsAsInputs,
      outputs: [],
    };
    const call = encodeJsonAbiFunctionCall(abi, args);
    return '0x' + call.slice(10); // strip selector
  }

  it('decodes basic scalar outputs', () => {
    const outputs = [
      { name: 'x', type: 'uint256' },
      { name: 'a', type: 'address' },
      { name: 'b', type: 'bool' },
      { name: 'raw', type: 'bytes' },
      { name: 's', type: 'string' },
    ] as const;

    const address = '0x' + '11'.repeat(20);
    const returnData = encodeReturnData(outputs, {
      x: 5n,
      a: address,
      b: true,
      raw: '0x1234',
      s: 'hi',
    });

    const decoded = decodeJsonAbiFunctionResult(
      { type: 'function', name: 'f', inputs: [], outputs: outputs as any },
      returnData
    );

    expect(decoded.x).toBe(5n);
    expect(decoded.a).toBe(address.toLowerCase());
    expect(decoded.b).toBe(true);
    expect(decoded.raw).toBe('0x1234');
    expect(decoded.s).toBe('hi');
  });

  it('decodes tuple outputs to object by component name', () => {
    const outputs = [
      {
        name: 'p',
        type: 'tuple',
        components: [
          { name: 'fee', type: 'uint24' },
          { name: 'recipient', type: 'address' },
        ],
      },
    ] as const;

    const recipient = '0x' + '22'.repeat(20);
    const returnData = encodeReturnData(outputs, {
      p: { fee: 3000n, recipient },
    });

    const decoded = decodeJsonAbiFunctionResult(
      { type: 'function', name: 'f', inputs: [], outputs: outputs as any },
      returnData
    );

    expect(decoded.p).toEqual({ fee: 3000n, recipient: recipient.toLowerCase() });
  });

  it('errors on empty/duplicate output names', () => {
    const outputs = [{ name: '', type: 'uint256' }] as any;
    const returnData = '0x' + pad32('0x1');
    expect(() =>
      decodeJsonAbiFunctionResult({ type: 'function', name: 'f', inputs: [], outputs }, returnData)
    ).toThrow(AbiDecodingError);
  });
});
