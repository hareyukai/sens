use std::{collections::HashMap, result};
use sdl2::event::Event;
use sdl2::EventPump;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::pixels::PixelFormatEnum;
use rand::Rng;
use bitflags::bitflags;

bitflags! {
    struct ProcessorStatus: u8 {
        const CARRY             = 0b0000_0001;
        const ZERO              = 0b0000_0010;
        const INTERRUPT_DISABLE = 0b0000_0100;
        const DECIMAL_MODE      = 0b0000_1000;
        const BREAK             = 0b0001_0000;
        const BREAK2            = 0b0010_0000;
        const OVERFLOW          = 0b0100_0000;
        const NEGATIVE          = 0b1000_0000;
    }
}

const STACK: u16 = 0x0100;
const STACK_RESET: u8 = 0xfd;

enum Opname {
    BRK,
    TAX,
    LDA
}

#[derive(Debug)]
enum AddressingMode {
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    IndirectX,
    IndirectY,
    Implied,
}

struct CPU {
    ra: u8,
    rx: u8,
    ry: u8,
    rs: u8,
    pc: u16,
    rp: ProcessorStatus,
    memory: [u8; 0xFFFF],
}

impl CPU {
    fn new() -> CPU {
        CPU {
            ra: 0,
            rx: 0,
            ry: 0,
            rs: STACK_RESET,
            pc: 0,
            rp: ProcessorStatus::BREAK2 | ProcessorStatus::INTERRUPT_DISABLE,
            memory: [0; 0xFFFF]
        }
    }

    fn get_operand_address(&self, mode: AddressingMode) -> u16 {
        match mode {
            AddressingMode::Immediate => self.pc,
            AddressingMode::ZeroPage => self.mem_read(self.pc) as u16,
            AddressingMode::Absolute => self.mem_read_u16(self.pc),
            AddressingMode::ZeroPageX => {
                self.mem_read(self.pc).wrapping_add(self.rx) as u16
            }
            AddressingMode::ZeroPageY => {
                self.mem_read(self.pc).wrapping_add(self.ry) as u16
            }
            AddressingMode::AbsoluteX => {
                self.mem_read_u16(self.pc).wrapping_add(self.rx as u16)
            }
            AddressingMode::AbsoluteY => {
                self.mem_read_u16(self.pc).wrapping_add(self.ry as u16)
            }
            AddressingMode::IndirectX => {
                let addr = self.mem_read(self.pc).wrapping_add(self.rx) as u16;
                let lo = self.mem_read(addr);
                let hi = self.mem_read(addr.wrapping_add(1));
                (hi as u16) << 8 | (lo as u16)
            }
            AddressingMode::IndirectY => {
                let addr = self.mem_read(self.pc) as u16;
                let lo = self.mem_read(addr);
                let hi = self.mem_read(addr.wrapping_add(1));
                let deref_base = (hi as u16) << 8 | (lo as u16);
                let deref = deref_base.wrapping_add(self.ry as u16);
                deref
            }
            AddressingMode::Implied => {
                panic!("mode {:?} is not supported", mode);
            }
        }
    }

    fn update_negative_flag(&mut self, reg: u8) {
        self.rp.set(ProcessorStatus::NEGATIVE, reg & 0b1000_0000 != 0);
    }

    fn update_zero_and_negative_flags(&mut self, reg: u8) {
        self.rp.set(ProcessorStatus::ZERO, reg == 0);
        self.update_negative_flag(reg);
    }

    fn set_reg_a(&mut self, val: u8) {
        self.ra = val;
        self.update_zero_and_negative_flags(self.ra);
    }

    fn add_to_reg_a(&mut self, val: u8) {
        let s = self.ra as u16 +
            val as u16 +
            if self.rp.contains(ProcessorStatus::CARRY) {1} else {0} as u16;
        self.rp.set(ProcessorStatus::CARRY, 0xff < s);
        let result = s as u8;
        self.rp.set(ProcessorStatus::OVERFLOW, (val ^ result) & (self.ra ^ result) & 0x80 != 0);
        self.set_reg_a(result);
    }

    fn adc(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.add_to_reg_a(val);
    }

    fn and(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.set_reg_a(self.ra & val);
    }

    fn asl_accumulator(&mut self) {
        let val = self.ra;
        self.rp.set(ProcessorStatus::CARRY, val & 0x80 != 0);
        self.set_reg_a(val << 1);
    }

    fn asl(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.rp.set(ProcessorStatus::CARRY, val & 0x80 != 0);
        let result = val << 1;
        self.mem_write(addr, result);
        self.update_zero_and_negative_flags(result);
    }

    fn branch(&mut self, condition: bool) {
        // if condition {
        //     let offset = self.mem_read(self.pc) as u16;
        //     self.pc = self.pc.wrapping_add(1).wrapping_add(offset);
        // }
        if condition {
            let jump: i8 = self.mem_read(self.pc) as i8;
            let jump_addr = self
                .pc
                .wrapping_add(1)
                .wrapping_add(jump as u16);

            self.pc = jump_addr;
        } else {
            self.pc += 1;
        }
    }

    fn bbc(&mut self) {
        self.branch(!self.rp.contains(ProcessorStatus::CARRY));
    }

    fn bcs(&mut self) {
        self.branch(self.rp.contains(ProcessorStatus::CARRY));
    }

    fn beq(&mut self) {
        self.branch(self.rp.contains(ProcessorStatus::ZERO));
    }

    fn bit(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.rp.set(ProcessorStatus::ZERO, self.ra & val == 0);
        self.rp.set(ProcessorStatus::OVERFLOW, val & 0b0100_0000 != 0);
        self.rp.set(ProcessorStatus::NEGATIVE, val & 0b1000_0000 != 0);
    }

    fn bmi(&mut self) {
        self.branch(self.rp.contains(ProcessorStatus::NEGATIVE));
    }

    fn bne(&mut self) {
        self.branch(!self.rp.contains(ProcessorStatus::ZERO));
    }

    fn bpl(&mut self) {
        self.branch(!self.rp.contains(ProcessorStatus::NEGATIVE));
    }

    fn bvc(&mut self) {
        self.branch(!self.rp.contains(ProcessorStatus::OVERFLOW));
    }

    fn bvs(&mut self) {
        self.branch(self.rp.contains(ProcessorStatus::OVERFLOW));
    }

    fn clc(&mut self) {
        self.rp.remove(ProcessorStatus::CARRY);
    }

    fn cld(&mut self) {
        self.rp.remove(ProcessorStatus::DECIMAL_MODE);
    }

    fn cli(&mut self) {
        self.rp.remove(ProcessorStatus::INTERRUPT_DISABLE);
    }

    fn clv(&mut self) {
        self.rp.remove(ProcessorStatus::OVERFLOW);
    }

    fn compare(&mut self, mode: AddressingMode, other: u8) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.rp.set(ProcessorStatus::CARRY, val <= other);
        self.update_zero_and_negative_flags(other.wrapping_sub(val));
    }

    fn cmp(&mut self, mode: AddressingMode) {
        self.compare(mode, self.ra);
    }

    fn cpx(&mut self, mode: AddressingMode) {
        self.compare(mode, self.rx);
    }

    fn cpy(&mut self, mode: AddressingMode) {
        self.compare(mode, self.ry);
    }

    fn dec(&mut self, mode:AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        let result = val.wrapping_sub(1);
        self.mem_write(addr, result);
        self.update_zero_and_negative_flags(result);
    }

    fn dex(&mut self) {
        self.rx = self.rx.wrapping_sub(1);
        self.update_zero_and_negative_flags(self.rx);
    }

    fn dey(&mut self) {
        self.ry = self.ry.wrapping_sub(1);
        self.update_zero_and_negative_flags(self.ry);
    }

    fn eor(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.set_reg_a(self.ra ^ val);
    }

    fn inc(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr).wrapping_add(1);
        self.mem_write(addr, val);
        self.update_zero_and_negative_flags(val);
    }

    fn inx(&mut self) {
        self.rx = self.rx.wrapping_add(1);
        self.update_zero_and_negative_flags(self.rx);
    }

    fn iny(&mut self) {
        self.ry = self.ry.wrapping_add(1);
        self.update_zero_and_negative_flags(self.ry);
    }

    fn jmp_absolute(&mut self) {
        let addr = self.mem_read_u16(self.pc);
        self.pc = addr;
    }

    fn jmp_indirect(&mut self) {
        let addr = self.mem_read_u16(self.pc);
        let indirect_addr = if addr & 0x00ff == 0x00ff {
            let lo = self.mem_read(addr);
            let hi = self.mem_read(addr & 0xff00);
            ((hi as u16) << 8) | (lo as u16)
        } else {
            self.mem_read_u16(addr)
        };
        self.pc = indirect_addr;
    }

    fn jsr(&mut self) {
        self.stack_push_u16(self.pc + 1);
        let addr = self.mem_read_u16(self.pc);
        self.pc = addr;
    }

    fn lda(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.set_reg_a(val);
    }

    fn ldx(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.rx = val;
        self.update_zero_and_negative_flags(self.rx);
    }

    fn ldy(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.ry = val;
        self.update_zero_and_negative_flags(self.ry);
    }

    fn lsr_accumulator(&mut self) {
        let val = self.ra;
        self.rp.set(ProcessorStatus::CARRY, val & 0x1 != 0);
        self.set_reg_a(val >> 1);
    }

    fn lsr(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.rp.set(ProcessorStatus::CARRY, val & 0x1 != 0);
        let result = val >> 1;
        self.mem_write(addr, result);
        self.update_zero_and_negative_flags(result);
    }

    fn ora(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.set_reg_a(self.ra | val);
    }

    fn pha(&mut self) {
        self.stack_push(self.ra);
    }

    fn php(&mut self) {
        let mut rp = self.rp.clone();
        rp.insert(ProcessorStatus::BREAK);
        rp.insert(ProcessorStatus::BREAK2);
        self.stack_push(rp.bits());
    }

    fn pla(&mut self) {
        let val = self.stack_pop();
        self.set_reg_a(val);
    }

    fn plp(&mut self) {
        self.rp.bits = self.stack_pop();
        self.rp.remove(ProcessorStatus::BREAK);
        self.rp.insert(ProcessorStatus::BREAK2);
    }

    fn rol_accumulator(&mut self) {
        let mut val = self.ra;
        let c = self.rp.contains(ProcessorStatus::CARRY);
        self.rp.set(ProcessorStatus::CARRY, val & 0x80 != 0);
        val = val << 1;
        if c {
            val = val | 1;
        }
        self.set_reg_a(val);
    }

    fn rol(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let mut val = self.mem_read(addr);
        let c = self.rp.contains(ProcessorStatus::CARRY);
        self.rp.set(ProcessorStatus::CARRY, val & 0x80 != 0);
        val = val << 1;
        if c {
            val = val | 1;
        }
        self.mem_write(addr, val);
        self.update_negative_flag(val);
    }

    fn ror_accumulator(&mut self) {
        let mut val = self.ra;
        let c = self.rp.contains(ProcessorStatus::CARRY);
        self.rp.set(ProcessorStatus::CARRY, val & 0x1 != 0);
        val = val >> 1;
        if c {
            val = val | 0b1000_0000;
        }
        self.set_reg_a(val);
    }

    fn ror(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let mut val = self.mem_read(addr);
        let c = self.rp.contains(ProcessorStatus::CARRY);
        self.rp.set(ProcessorStatus::CARRY, val & 0x1 != 0);
        val = val >> 1;
        if c {
            val = val | 0b1000_0000;
        }
        self.mem_write(addr, val);
        self.update_negative_flag(val);

    }

    fn rti(&mut self) {
        self.rp.bits = self.stack_pop();
        self.rp.remove(ProcessorStatus::BREAK);
        self.rp.insert(ProcessorStatus::BREAK2);
        self.pc = self.stack_pop_u16();
    }

    fn rts(&mut self) {
        self.pc = self.stack_pop_u16() + 1;
    }

    fn sbc(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        let val = self.mem_read(addr);
        self.add_to_reg_a((val as i8).wrapping_neg().wrapping_sub(1) as u8);
    }

    fn sec(&mut self) {
        self.rp.insert(ProcessorStatus::CARRY);
    }

    fn sed(&mut self) {
        self.rp.insert(ProcessorStatus::DECIMAL_MODE);
    }

    fn sei(&mut self) {
        self.rp.insert(ProcessorStatus::INTERRUPT_DISABLE);
    }

    fn sta(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.ra);
    }

    fn stx(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.rx);
    }

    fn sty(&mut self, mode: AddressingMode) {
        let addr = self.get_operand_address(mode);
        self.mem_write(addr, self.ry);
    }

    fn tax(&mut self) {
        self.rx = self.ra;
        self.update_zero_and_negative_flags(self.rx);
    }

    fn tay(&mut self) {
        self.ry = self.ra;
        self.update_zero_and_negative_flags(self.ry);
    }

    fn tsx(&mut self) {
        self.rx = self.rs;
        self.update_zero_and_negative_flags(self.rx);
    }

    fn txa(&mut self) {
        self.ra = self.rx;
        self.update_zero_and_negative_flags(self.ra);
    }

    fn txs(&mut self) {
        self.rs = self.rx;
    }

    fn tya(&mut self) {
        self.ra = self.ry;
        self.update_zero_and_negative_flags(self.ra);
    }

    fn mem_read(&self, addr: u16) -> u8 {
        self.memory[addr as usize]
    }

    fn mem_write(&mut self, addr: u16, val: u8) {
        self.memory[addr as usize] = val;
    }

    fn mem_read_u16(&self, pos: u16) -> u16 {
        let lo = self.mem_read(pos);
        let hi = self.mem_read(pos + 1);
        u16::from_le_bytes([lo, hi])
    }

    fn mem_write_u16(&mut self, pos: u16, val: u16) {
        let hi = (val >> 8) as u8;
        let lo = (val & 0xff) as u8;
        self.mem_write(pos, lo);
        self.mem_write(pos + 1, hi);
    }

    fn stack_pop(&mut self) -> u8 {
        self.rs = self.rs.wrapping_add(1);
        self.mem_read(STACK + self.rs as u16)
    }

    fn stack_pop_u16(&mut self) -> u16 {
        let lo = self.stack_pop() as u16;
        let hi = self.stack_pop() as u16;
        (hi << 8) | lo
    }

    fn stack_push(&mut self, val: u8) {
        self.mem_write(STACK + self.rs as u16, val);
        self.rs = self.rs.wrapping_sub(1);
    }

    fn stack_push_u16(&mut self, val: u16) {
        let hi = (val >> 8) as u8;
        let lo = (val & 0xff) as u8;
        self.stack_push(hi);
        self.stack_push(lo);
    }

    fn reset(&mut self) {
        self.ra = 0;
        self.rx = 0;
        self.ry = 0;
        self.rs = STACK_RESET;
        self.rp = ProcessorStatus::BREAK2 | ProcessorStatus::INTERRUPT_DISABLE;
        self.pc = self.mem_read_u16(0xFFFC);
    }

    fn load(&mut self, program: Vec<u8>) {
        self.memory[0x0600 .. (0x0600 + program.len())].copy_from_slice(&program[..]);
        self.mem_write_u16(0xFFFC, 0x0600)
    }

    fn load_and_run(&mut self, program: Vec<u8>) {
        self.load(program);
        self.reset();
        self.run();
    }

    fn run(&mut self) {
        self.run_with_callback(|_| {});
    }

    fn run_with_callback<F>(&mut self, mut callback: F) where F: FnMut(&mut CPU) {

        loop {


            let opscode = self.mem_read(self.pc);

            // println!("{:x} {:x} {:x}", self.pc, self.mem_read(self.pc + 1), opscode);

            self.pc += 1;

            match opscode {
                0x69 => {
                    self.adc(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0x65 => {
                    self.adc(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x75 => {
                    self.adc(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x6d => {
                    self.adc(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x7d => {
                    self.adc(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x79 => {
                    self.adc(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x61 => {
                    self.adc(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0x71 => {
                    self.adc(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0x29 => {
                    self.and(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0x25 => {
                    self.and(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x35 => {
                    self.and(AddressingMode::ZeroPageX);
                    self.pc += 1
                }
                0x2d => {
                    self.and(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x3d => {
                    self.and(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x39 => {
                    self.and(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0x21 => {
                    self.and(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0x31 => {
                    self.and(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0x0a => {
                    self.asl_accumulator();
                }
                0x06 => {
                    self.asl(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x16 => {
                    self.asl(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x0e => {
                    self.asl(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x1e => {
                    self.asl(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x90 => {
                    self.bbc();
                }
                0xb0 => {
                    self.bcs();
                }
                0xf0 => {
                    self.beq();
                }
                0x24 => {
                    self.bit(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x2c => {
                    self.bit(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x30 => {
                    self.bmi();
                }
                0xd0 => {
                    self.bne();
                }
                0x10 => {
                    self.bpl();
                }
                0x50 => {
                    self.bvc();
                }
                0x70 => {
                    self.bvs();
                }
                0x18 => {
                    self.clc();
                }
                0xd8 => {
                    self.cld();
                }
                0x58 => {
                    self.cli();
                }
                0xb8 => {
                    self.clv();
                }
                0xc9 => {
                    self.cmp(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xc5 => {
                    self.cmp(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xd5 => {
                    self.cmp(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xcd => {
                    self.cmp(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xdd => {
                    self.cmp(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0xd9 => {
                    self.cmp(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0xc1 => {
                    self.cmp(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0xd1 => {
                    self.cmp(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0xe0 => {
                    self.cpx(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xe4 => {
                    self.cpx(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xec => {
                    self.cpx(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xc0 => {
                    self.cpy(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xc4 => {
                    self.cpy(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xcc => {
                    self.cpy(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xc6 => {
                    self.dec(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xd6 => {
                    self.dec(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xce => {
                    self.dec(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xde => {
                    self.dec(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0xca => {
                    self.dex();
                }
                0x88 => {
                    self.dey();
                }
                0x49 => {
                    self.eor(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0x45 => {
                    self.eor(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x55 => {
                    self.eor(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x4d => {
                    self.eor(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x5d => {
                    self.eor(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x59 => {
                    self.eor(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0x41 => {
                    self.eor(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0x51 => {
                    self.eor(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0xe6 => {
                    self.inc(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xf6 => {
                    self.inc(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xee => {
                    self.inc(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xfe => {
                    self.inc(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0xe8 => self.inx(),
                0xc8 => self.iny(),
                0x4c => {
                    self.jmp_absolute();
                }
                0x6c => {
                    self.jmp_indirect();
                }
                0x20 => {
                    self.jsr();
                }
                0xa9 => {
                    self.lda(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xa5 => {
                    self.lda(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xb5 => {
                    self.lda(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xad => {
                    self.lda(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xbd => {
                    self.lda(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0xb9 => {
                    self.lda(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0xa1 => {
                    self.lda(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0xb1 => {
                    self.lda(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0xa2 => {
                    self.ldx(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xa6 => {
                    self.ldx(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xb6 => {
                    self.ldx(AddressingMode::ZeroPageY);
                    self.pc += 1;
                }
                0xae => {
                    self.ldx(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xbe => {
                    self.ldx(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0xa0 => {
                    self.ldy(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xa4 => {
                    self.ldy(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xb4 => {
                    self.ldy(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xac => {
                    self.ldy(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xbc => {
                    self.ldy(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x4a => {
                    self.lsr_accumulator();
                }
                0x46 => {
                    self.lsr(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x56 => {
                    self.lsr(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x4e => {
                    self.lsr(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x5e => {
                    self.lsr(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x09 => {
                    self.ora(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0x05 => {
                    self.ora(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x15 => {
                    self.ora(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x0d => {
                    self.ora(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x1d => {
                    self.ora(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x19 => {
                    self.ora(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0x01 => {
                    self.ora(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0x11 => {
                    self.ora(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0x48 => {
                    self.pha();
                }
                0x08 => {
                    self.php();
                }
                0x68 => {
                    self.pla();
                }
                0x28 => {
                    self.plp();
                }
                0x2a => {
                    self.rol_accumulator();
                }
                0x26 => {
                    self.rol(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x36 => {
                    self.rol(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x2e => {
                    self.rol(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x3e => {
                    self.rol(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x6a => {
                    self.ror_accumulator();
                }
                0x66 => {
                    self.ror(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x76 => {
                    self.ror(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x6e => {
                    self.ror(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x7e => {
                    self.ror(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x40 => {
                    self.rti();
                }
                0x60 => {
                    self.rts();
                }
                0xe9 => {
                    self.sbc(AddressingMode::Immediate);
                    self.pc += 1;
                }
                0xe5 => {
                    self.sbc(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0xf5 => {
                    self.sbc(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0xed => {
                    self.sbc(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xfd => {
                    self.sbc(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0xf9 => {
                    self.sbc(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0xe1 => {
                    self.sbc(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0xf1 => {
                    self.sbc(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0x38 => {
                    self.sec();
                }
                0xf8 => {
                    self.sed();
                }
                0x78 => {
                    self.sei();
                }
                0x85 => {
                    self.sta(AddressingMode::ZeroPage);
                    self.pc += 1
                }
                0x95 => {
                    self.sta(AddressingMode::ZeroPageX);
                    self.pc += 1
                }
                0x8d => {
                    self.sta(AddressingMode::Absolute);
                    self.pc += 2
                }
                0x9d => {
                    self.sta(AddressingMode::AbsoluteX);
                    self.pc += 2;
                }
                0x99 => {
                    self.sta(AddressingMode::AbsoluteY);
                    self.pc += 2;
                }
                0x81 => {
                    self.sta(AddressingMode::IndirectX);
                    self.pc += 1;
                }
                0x91 => {
                    self.sta(AddressingMode::IndirectY);
                    self.pc += 1;
                }
                0x86 => {
                    self.stx(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x96 => {
                    self.stx(AddressingMode::ZeroPageY);
                    self.pc += 1;
                }
                0x8e => {
                    self.stx(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0x84 => {
                    self.sty(AddressingMode::ZeroPage);
                    self.pc += 1;
                }
                0x94 => {
                    self.sty(AddressingMode::ZeroPageX);
                    self.pc += 1;
                }
                0x8c => {
                    self.sty(AddressingMode::Absolute);
                    self.pc += 2;
                }
                0xaa => self.tax(),

                0xa8 => self.tay(),

                0xba => self.tsx(),

                0x8a => self.txa(),

                0x9a => self.txs(),

                0x98 => self.tya(),

                0xea => {

                }

                0x00 => return,
                _ => todo!()
            }

            callback(self);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_adc_from_memory() {
        let mut cpu = CPU::new();
        cpu.load_and_run(vec![0x69, 0x13, 0x00]);
        println!("{}",cpu.ra);
        assert_eq!(cpu.ra, 0x13);
    }

    #[test]
    fn test_lda_immediate() {
        let mut cpu = CPU::new();
        cpu.load_and_run(vec![0xA9, 0x17, 0x00]);
        assert_eq!(cpu.ra, 0x17);
    }

    #[test]
    fn test_lda_from_memory() {
        let mut cpu = CPU::new();
        cpu.mem_write(0x10, 0x55);
        cpu.load_and_run(vec![0xa5, 0x10, 0x00]);
        assert_eq!(cpu.ra, 0x55);
    }


    #[test]
    fn test_0xa9_lda_immidiate_load_data() {
        let mut cpu = CPU::new();
        cpu.load_and_run(vec![0xa9, 0x05, 0x00]);
        assert_eq!(cpu.ra, 5);
        assert!(!cpu.rp.contains(ProcessorStatus::ZERO));
        assert!(!cpu.rp.contains(ProcessorStatus::NEGATIVE));
    }

    #[test]
    fn test_0xaa_tax_move_a_to_x() {
        let mut cpu = CPU::new();
        cpu.ra = 10;
        cpu.load_and_run(vec![0xaa, 0x00]);

        assert_eq!(cpu.rx, 10)
    }

    #[test]
    fn test_5_ops_working_together() {
        let mut cpu = CPU::new();
        cpu.load_and_run(vec![0xa9, 0xc0, 0xaa, 0xe8, 0x00]);

        assert_eq!(cpu.rx, 0xc1)
    }

    #[test]
    fn test_inx_overflow() {
        let mut cpu = CPU::new();
        cpu.load_and_run(vec![0xe8, 0xe8, 0x00]);
        assert_eq!(cpu.rx, 2);
    }

}

fn color(byte: u8) -> Color {
    match byte {
        0 => sdl2::pixels::Color::BLACK,
        1 => sdl2::pixels::Color::WHITE,
        2 | 9 => sdl2::pixels::Color::GREY,
        3 | 10 => sdl2::pixels::Color::RED,
        4 | 11 => sdl2::pixels::Color::GREEN,
        5 | 12 => sdl2::pixels::Color::BLUE,
        6 | 13 => sdl2::pixels::Color::MAGENTA,
        7 | 14 => sdl2::pixels::Color::YELLOW,
        _ => sdl2::pixels::Color::CYAN,
    }
}

fn read_screen_state(cpu: &CPU, frame: &mut [u8; 32 * 3 * 32]) -> bool {
    let mut frame_idx = 0;
    let mut update = false;
    for i in 0x0200..0x600 {
        let color_idx = cpu.mem_read(i as u16);
        let (b1, b2, b3) = color(color_idx).rgb();
        if frame[frame_idx] != b1 || frame[frame_idx + 1] != b2 || frame[frame_idx + 2] != b3 {
            frame[frame_idx] = b1;
            frame[frame_idx + 1] = b2;
            frame[frame_idx + 2] = b3;
            update = true;
        }
        frame_idx += 3;
    }
    update
}

fn handle_user_input(cpu: &mut CPU, event_pump: &mut EventPump) {
    for event in event_pump.poll_iter() {
        match event {
            Event::Quit { .. } | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                std::process::exit(0)
            },
            Event::KeyDown { keycode: Some(Keycode::W), .. } => {
                cpu.mem_write(0xff, 0x77);
            },
            Event::KeyDown { keycode: Some(Keycode::S), .. } => {
                cpu.mem_write(0xff, 0x73);
            },
            Event::KeyDown { keycode: Some(Keycode::A), .. } => {
                cpu.mem_write(0xff, 0x61);
            },
            Event::KeyDown { keycode: Some(Keycode::D), .. } => {
                cpu.mem_write(0xff, 0x64);
            }
            _ => {/* do nothing */}
        }
    }
}

fn main() {
    // init sdl2
    let sdl_context = sdl2::init().unwrap();
    let video_subsystem = sdl_context.video().unwrap();
    let window = video_subsystem
        .window("Snake game", (32.0 * 10.0) as u32, (32.0 * 10.0) as u32)
        .position_centered()
        .build().unwrap();

    let mut canvas = window.into_canvas().present_vsync().build().unwrap();
    let mut event_pump = sdl_context.event_pump().unwrap();
    canvas.set_scale(10.0, 10.0).unwrap();

    let creator = canvas.texture_creator();
    let mut texture = creator
        .create_texture_target(PixelFormatEnum::RGB24, 32, 32).unwrap();


    let game_code = vec![
        0x20, 0x06, 0x06, 0x20, 0x38, 0x06, 0x20, 0x0d, 0x06, 0x20, 0x2a, 0x06, 0x60, 0xa9, 0x02,
        0x85, 0x02, 0xa9, 0x04, 0x85, 0x03, 0xa9, 0x11, 0x85, 0x10, 0xa9, 0x10, 0x85, 0x12, 0xa9,
        0x0f, 0x85, 0x14, 0xa9, 0x04, 0x85, 0x11, 0x85, 0x13, 0x85, 0x15, 0x60, 0xa5, 0xfe, 0x85,
        0x00, 0xa5, 0xfe, 0x29, 0x03, 0x18, 0x69, 0x02, 0x85, 0x01, 0x60, 0x20, 0x4d, 0x06, 0x20,
        0x8d, 0x06, 0x20, 0xc3, 0x06, 0x20, 0x19, 0x07, 0x20, 0x20, 0x07, 0x20, 0x2d, 0x07, 0x4c,
        0x38, 0x06, 0xa5, 0xff, 0xc9, 0x77, 0xf0, 0x0d, 0xc9, 0x64, 0xf0, 0x14, 0xc9, 0x73, 0xf0,
        0x1b, 0xc9, 0x61, 0xf0, 0x22, 0x60, 0xa9, 0x04, 0x24, 0x02, 0xd0, 0x26, 0xa9, 0x01, 0x85,
        0x02, 0x60, 0xa9, 0x08, 0x24, 0x02, 0xd0, 0x1b, 0xa9, 0x02, 0x85, 0x02, 0x60, 0xa9, 0x01,
        0x24, 0x02, 0xd0, 0x10, 0xa9, 0x04, 0x85, 0x02, 0x60, 0xa9, 0x02, 0x24, 0x02, 0xd0, 0x05,
        0xa9, 0x08, 0x85, 0x02, 0x60, 0x60, 0x20, 0x94, 0x06, 0x20, 0xa8, 0x06, 0x60, 0xa5, 0x00,
        0xc5, 0x10, 0xd0, 0x0d, 0xa5, 0x01, 0xc5, 0x11, 0xd0, 0x07, 0xe6, 0x03, 0xe6, 0x03, 0x20,
        0x2a, 0x06, 0x60, 0xa2, 0x02, 0xb5, 0x10, 0xc5, 0x10, 0xd0, 0x06, 0xb5, 0x11, 0xc5, 0x11,
        0xf0, 0x09, 0xe8, 0xe8, 0xe4, 0x03, 0xf0, 0x06, 0x4c, 0xaa, 0x06, 0x4c, 0x35, 0x07, 0x60,
        0xa6, 0x03, 0xca, 0x8a, 0xb5, 0x10, 0x95, 0x12, 0xca, 0x10, 0xf9, 0xa5, 0x02, 0x4a, 0xb0,
        0x09, 0x4a, 0xb0, 0x19, 0x4a, 0xb0, 0x1f, 0x4a, 0xb0, 0x2f, 0xa5, 0x10, 0x38, 0xe9, 0x20,
        0x85, 0x10, 0x90, 0x01, 0x60, 0xc6, 0x11, 0xa9, 0x01, 0xc5, 0x11, 0xf0, 0x28, 0x60, 0xe6,
        0x10, 0xa9, 0x1f, 0x24, 0x10, 0xf0, 0x1f, 0x60, 0xa5, 0x10, 0x18, 0x69, 0x20, 0x85, 0x10,
        0xb0, 0x01, 0x60, 0xe6, 0x11, 0xa9, 0x06, 0xc5, 0x11, 0xf0, 0x0c, 0x60, 0xc6, 0x10, 0xa5,
        0x10, 0x29, 0x1f, 0xc9, 0x1f, 0xf0, 0x01, 0x60, 0x4c, 0x35, 0x07, 0xa0, 0x00, 0xa5, 0xfe,
        0x91, 0x00, 0x60, 0xa6, 0x03, 0xa9, 0x00, 0x81, 0x10, 0xa2, 0x00, 0xa9, 0x01, 0x81, 0x10,
        0x60, 0xa6, 0xff, 0xea, 0xea, 0xca, 0xd0, 0xfb, 0x60,
    ];


    //load the game
    let mut cpu = CPU::new();
    cpu.load(game_code);
    cpu.reset();

    let mut screen_state = [0 as u8; 32 * 3 * 32];
    let mut rng = rand::thread_rng();

    // run the game cycle
    cpu.run_with_callback(move |cpu| {
        handle_user_input(cpu, &mut event_pump);

        cpu.mem_write(0xfe, rng.gen_range(1.. 16));

        if read_screen_state(cpu, &mut screen_state) {
            texture.update(None, &screen_state, 32 * 3).unwrap();

            canvas.copy(&texture, None, None).unwrap();

            canvas.present();
        }

        ::std::thread::sleep(std::time::Duration::new(0, 70_000));
    });

}
