/**
 * Borsh serialization for Solana instructions
 * 
 * Borsh (Binary Object Representation Serializer for Hashing) is the
 * standard serialization format for Solana program data.
 * 
 * This is a minimal implementation covering common types.
 */

/**
 * Borsh writer for serializing data
 */
export class BorshWriter {
  private buffer: Uint8Array;
  private offset: number;

  constructor(initialSize: number = 256) {
    this.buffer = new Uint8Array(initialSize);
    this.offset = 0;
  }

  private ensureCapacity(additional: number): void {
    const required = this.offset + additional;
    if (required > this.buffer.length) {
      const newSize = Math.max(this.buffer.length * 2, required);
      const newBuffer = new Uint8Array(newSize);
      newBuffer.set(this.buffer);
      this.buffer = newBuffer;
    }
  }

  /**
   * Write a u8 (unsigned 8-bit integer)
   */
  writeU8(value: number): this {
    this.ensureCapacity(1);
    this.buffer[this.offset++] = value & 0xff;
    return this;
  }

  /**
   * Write a u16 (unsigned 16-bit integer, little-endian)
   */
  writeU16(value: number): this {
    this.ensureCapacity(2);
    this.buffer[this.offset++] = value & 0xff;
    this.buffer[this.offset++] = (value >> 8) & 0xff;
    return this;
  }

  /**
   * Write a u32 (unsigned 32-bit integer, little-endian)
   */
  writeU32(value: number): this {
    this.ensureCapacity(4);
    this.buffer[this.offset++] = value & 0xff;
    this.buffer[this.offset++] = (value >> 8) & 0xff;
    this.buffer[this.offset++] = (value >> 16) & 0xff;
    this.buffer[this.offset++] = (value >> 24) & 0xff;
    return this;
  }

  /**
   * Write a u64 (unsigned 64-bit integer, little-endian)
   */
  writeU64(value: bigint | number): this {
    const bigValue = BigInt(value);
    this.ensureCapacity(8);
    for (let i = 0; i < 8; i++) {
      this.buffer[this.offset++] = Number((bigValue >> BigInt(i * 8)) & BigInt(0xff));
    }
    return this;
  }

  /**
   * Write a u128 (unsigned 128-bit integer, little-endian)
   */
  writeU128(value: bigint): this {
    this.ensureCapacity(16);
    for (let i = 0; i < 16; i++) {
      this.buffer[this.offset++] = Number((value >> BigInt(i * 8)) & BigInt(0xff));
    }
    return this;
  }

  /**
   * Write an i8 (signed 8-bit integer)
   */
  writeI8(value: number): this {
    return this.writeU8(value < 0 ? value + 256 : value);
  }

  /**
   * Write an i16 (signed 16-bit integer, little-endian)
   */
  writeI16(value: number): this {
    return this.writeU16(value < 0 ? value + 65536 : value);
  }

  /**
   * Write an i32 (signed 32-bit integer, little-endian)
   */
  writeI32(value: number): this {
    return this.writeU32(value < 0 ? value + 4294967296 : value);
  }

  /**
   * Write an i64 (signed 64-bit integer, little-endian)
   */
  writeI64(value: bigint | number): this {
    const bigValue = BigInt(value);
    const unsigned = bigValue < 0n ? bigValue + (1n << 64n) : bigValue;
    return this.writeU64(unsigned);
  }

  /**
   * Write a boolean (1 byte)
   */
  writeBool(value: boolean): this {
    return this.writeU8(value ? 1 : 0);
  }

  /**
   * Write raw bytes
   */
  writeBytes(bytes: Uint8Array): this {
    this.ensureCapacity(bytes.length);
    this.buffer.set(bytes, this.offset);
    this.offset += bytes.length;
    return this;
  }

  /**
   * Write a fixed-size array (no length prefix)
   */
  writeFixedArray(bytes: Uint8Array): this {
    return this.writeBytes(bytes);
  }

  /**
   * Write a dynamic array with u32 length prefix
   */
  writeArray<T>(items: T[], writeItem: (item: T) => void): this {
    this.writeU32(items.length);
    for (const item of items) {
      writeItem(item);
    }
    return this;
  }

  /**
   * Write a string (u32 length prefix + UTF-8 bytes)
   */
  writeString(value: string): this {
    const bytes = new TextEncoder().encode(value);
    this.writeU32(bytes.length);
    return this.writeBytes(bytes);
  }

  /**
   * Write a public key (32 bytes)
   */
  writePublicKey(bytes: Uint8Array): this {
    if (bytes.length !== 32) {
      throw new Error(`Invalid public key length: ${bytes.length}`);
    }
    return this.writeBytes(bytes);
  }

  /**
   * Write an Option<T> (1 byte discriminant + optional value)
   */
  writeOption<T>(value: T | null | undefined, writeValue: (v: T) => void): this {
    if (value === null || value === undefined) {
      return this.writeU8(0);
    }
    this.writeU8(1);
    writeValue(value);
    return this;
  }

  /**
   * Get the serialized bytes
   */
  toBytes(): Uint8Array {
    return this.buffer.slice(0, this.offset);
  }
}

/**
 * Borsh reader for deserializing data
 */
export class BorshReader {
  private buffer: Uint8Array;
  private offset: number;

  constructor(buffer: Uint8Array) {
    this.buffer = buffer;
    this.offset = 0;
  }

  private checkCapacity(size: number): void {
    if (this.offset + size > this.buffer.length) {
      throw new Error(`Buffer overflow: need ${size} bytes at offset ${this.offset}, have ${this.buffer.length}`);
    }
  }

  readU8(): number {
    this.checkCapacity(1);
    return this.buffer[this.offset++];
  }

  readU16(): number {
    this.checkCapacity(2);
    const value = this.buffer[this.offset] | (this.buffer[this.offset + 1] << 8);
    this.offset += 2;
    return value;
  }

  readU32(): number {
    this.checkCapacity(4);
    const value = 
      this.buffer[this.offset] |
      (this.buffer[this.offset + 1] << 8) |
      (this.buffer[this.offset + 2] << 16) |
      (this.buffer[this.offset + 3] << 24);
    this.offset += 4;
    return value >>> 0;  // Convert to unsigned
  }

  readU64(): bigint {
    this.checkCapacity(8);
    let value = BigInt(0);
    for (let i = 0; i < 8; i++) {
      value |= BigInt(this.buffer[this.offset + i]) << BigInt(i * 8);
    }
    this.offset += 8;
    return value;
  }

  readBool(): boolean {
    return this.readU8() !== 0;
  }

  readBytes(length: number): Uint8Array {
    this.checkCapacity(length);
    const bytes = this.buffer.slice(this.offset, this.offset + length);
    this.offset += length;
    return bytes;
  }

  readPublicKey(): Uint8Array {
    return this.readBytes(32);
  }

  readString(): string {
    const length = this.readU32();
    const bytes = this.readBytes(length);
    return new TextDecoder().decode(bytes);
  }

  readOption<T>(readValue: () => T): T | null {
    const discriminant = this.readU8();
    if (discriminant === 0) {
      return null;
    }
    return readValue();
  }

  get remaining(): number {
    return this.buffer.length - this.offset;
  }
}

/**
 * Serialize SPL Token transfer instruction data
 * Format: [instruction_id (u8), amount (u64)]
 */
export function serializeSplTransfer(amount: bigint): Uint8Array {
  const writer = new BorshWriter(9);
  writer.writeU8(3);  // Transfer instruction = 3
  writer.writeU64(amount);
  return writer.toBytes();
}

/**
 * Serialize SPL Token transferChecked instruction data
 * Format: [instruction_id (u8), amount (u64), decimals (u8)]
 */
export function serializeSplTransferChecked(amount: bigint, decimals: number): Uint8Array {
  const writer = new BorshWriter(10);
  writer.writeU8(12);  // TransferChecked instruction = 12
  writer.writeU64(amount);
  writer.writeU8(decimals);
  return writer.toBytes();
}

/**
 * Serialize SPL Token approve instruction data
 * Format: [instruction_id (u8), amount (u64)]
 */
export function serializeSplApprove(amount: bigint): Uint8Array {
  const writer = new BorshWriter(9);
  writer.writeU8(4);  // Approve instruction = 4
  writer.writeU64(amount);
  return writer.toBytes();
}
