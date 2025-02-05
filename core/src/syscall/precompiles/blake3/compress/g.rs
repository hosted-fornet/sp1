use p3_field::Field;

use sp1_derive::AlignedBorrow;
use std::mem::size_of;

use crate::air::SP1AirBuilder;
use crate::air::Word;
use crate::air::WORD_SIZE;
use crate::operations::AddOperation;
use crate::operations::FixedRotateRightOperation;
use crate::operations::XorOperation;
use crate::runtime::ExecutionRecord;

use super::g_func;
/// A set of columns needed to compute the `g` of the input state.
///  ``` ignore
/// fn g(state: &mut BlockWords, a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
///     state[a] = state[a].wrapping_add(state[b]).wrapping_add(x);
///     state[d] = (state[d] ^ state[a]).rotate_right(16);
///     state[c] = state[c].wrapping_add(state[d]);
///     state[b] = (state[b] ^ state[c]).rotate_right(12);
///     state[a] = state[a].wrapping_add(state[b]).wrapping_add(y);
///     state[d] = (state[d] ^ state[a]).rotate_right(8);
///     state[c] = state[c].wrapping_add(state[d]);
///     state[b] = (state[b] ^ state[c]).rotate_right(7);
/// }
///  ```
#[derive(AlignedBorrow, Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct GOperation<T> {
    pub a_plus_b: AddOperation<T>,
    pub a_plus_b_plus_x: AddOperation<T>,
    pub d_xor_a: XorOperation<T>,
    // Rotate right by 16 bits by just shifting bytes.
    pub c_plus_d: AddOperation<T>,
    pub b_xor_c: XorOperation<T>,
    pub b_xor_c_rotate_right_12: FixedRotateRightOperation<T>,
    pub a_plus_b_2: AddOperation<T>,
    pub a_plus_b_2_add_y: AddOperation<T>,
    // Rotate right by 8 bits by just shifting bytes.
    pub d_xor_a_2: XorOperation<T>,
    pub c_plus_d_2: AddOperation<T>,
    pub b_xor_c_2: XorOperation<T>,
    pub b_xor_c_2_rotate_right_7: FixedRotateRightOperation<T>,
    /// `state[a]`, `state[b]`, `state[c]`, `state[d]` after all the steps.
    pub result: [Word<T>; 4],
}

impl<F: Field> GOperation<F> {
    pub fn populate(&mut self, record: &mut ExecutionRecord, input: [u32; 6]) -> [u32; 4] {
        let mut a = input[0];
        let mut b = input[1];
        let mut c = input[2];
        let mut d = input[3];
        let x = input[4];
        let y = input[5];

        // First 4 steps.
        {
            // a = a + b + x.
            a = self.a_plus_b.populate(record, a, b);
            a = self.a_plus_b_plus_x.populate(record, a, x);

            // d = (d ^ a).rotate_right(16).
            d = self.d_xor_a.populate(record, d, a);
            d = d.rotate_right(16);

            // c = c + d.
            c = self.c_plus_d.populate(record, c, d);

            // b = (b ^ c).rotate_right(12).
            b = self.b_xor_c.populate(record, b, c);
            b = self.b_xor_c_rotate_right_12.populate(record, b, 12);
        }

        // Second 4 steps.
        {
            // a = a + b + y.
            a = self.a_plus_b_2.populate(record, a, b);
            a = self.a_plus_b_2_add_y.populate(record, a, y);

            // d = (d ^ a).rotate_right(8).
            d = self.d_xor_a_2.populate(record, d, a);
            d = d.rotate_right(8);

            // c = c + d.
            c = self.c_plus_d_2.populate(record, c, d);

            // b = (b ^ c).rotate_right(7).
            b = self.b_xor_c_2.populate(record, b, c);
            b = self.b_xor_c_2_rotate_right_7.populate(record, b, 7);
        }

        let result = [a, b, c, d];
        assert_eq!(result, g_func(input));
        self.result = result.map(Word::from);
        result
    }

    pub fn eval<AB: SP1AirBuilder>(
        builder: &mut AB,
        input: [Word<AB::Var>; 6],
        cols: GOperation<AB::Var>,
        is_real: AB::Var,
    ) {
        builder.assert_bool(is_real);
        let mut a = input[0];
        let mut b = input[1];
        let mut c = input[2];
        let mut d = input[3];
        let x = input[4];
        let y = input[5];

        // First 4 steps.
        {
            // a = a + b + x.
            AddOperation::<AB::F>::eval(builder, a, b, cols.a_plus_b, is_real);
            a = cols.a_plus_b.value;
            AddOperation::<AB::F>::eval(builder, a, x, cols.a_plus_b_plus_x, is_real);
            a = cols.a_plus_b_plus_x.value;

            // d = (d ^ a).rotate_right(16).
            XorOperation::<AB::F>::eval(builder, d, a, cols.d_xor_a, is_real);
            d = cols.d_xor_a.value;
            // Rotate right by 16 bits.
            d = Word([d[2], d[3], d[0], d[1]]);

            // c = c + d.
            AddOperation::<AB::F>::eval(builder, c, d, cols.c_plus_d, is_real);
            c = cols.c_plus_d.value;

            // b = (b ^ c).rotate_right(12).
            XorOperation::<AB::F>::eval(builder, b, c, cols.b_xor_c, is_real);
            b = cols.b_xor_c.value;
            FixedRotateRightOperation::<AB::F>::eval(
                builder,
                b,
                12,
                cols.b_xor_c_rotate_right_12,
                is_real,
            );
            b = cols.b_xor_c_rotate_right_12.value;
        }

        // Second 4 steps.
        {
            // a = a + b + y.
            AddOperation::<AB::F>::eval(builder, a, b, cols.a_plus_b_2, is_real);
            a = cols.a_plus_b_2.value;
            AddOperation::<AB::F>::eval(builder, a, y, cols.a_plus_b_2_add_y, is_real);
            a = cols.a_plus_b_2_add_y.value;

            // d = (d ^ a).rotate_right(8).
            XorOperation::<AB::F>::eval(builder, d, a, cols.d_xor_a_2, is_real);
            d = cols.d_xor_a_2.value;
            // Rotate right by 8 bits.
            d = Word([d[1], d[2], d[3], d[0]]);

            // c = c + d.
            AddOperation::<AB::F>::eval(builder, c, d, cols.c_plus_d_2, is_real);
            c = cols.c_plus_d_2.value;

            // b = (b ^ c).rotate_right(7).
            XorOperation::<AB::F>::eval(builder, b, c, cols.b_xor_c_2, is_real);
            b = cols.b_xor_c_2.value;
            FixedRotateRightOperation::<AB::F>::eval(
                builder,
                b,
                7,
                cols.b_xor_c_2_rotate_right_7,
                is_real,
            );
            b = cols.b_xor_c_2_rotate_right_7.value;
        }

        let results = [a, b, c, d];
        for i in 0..4 {
            for j in 0..WORD_SIZE {
                builder.assert_eq(cols.result[i][j], results[i][j]);
            }
        }
        // Degree 3 constraint to avoid "OodEvaluationMismatch".
        builder.assert_zero(is_real * is_real * is_real - is_real * is_real * is_real);
    }
}
