use crate::air::MachineAir;
use crate::air::SP1AirBuilder;
use crate::memory::MemoryCols;
use crate::memory::MemoryReadCols;
use crate::memory::MemoryWriteCols;
use crate::operations::field::field_op::FieldOpCols;
use crate::operations::field::field_op::FieldOperation;
use crate::operations::field::params::NUM_LIMBS;
use crate::runtime::ExecutionRecord;
use crate::runtime::Register;
use crate::runtime::Syscall;
use crate::syscall::precompiles::create_ec_add_event;
use crate::syscall::precompiles::SyscallContext;
use crate::utils::ec::weierstrass::WeierstrassParameters;
use crate::utils::ec::AffinePoint;
use crate::utils::ec::EllipticCurve;
use crate::utils::ec::NUM_WORDS_EC_POINT;
use crate::utils::ec::NUM_WORDS_FIELD_ELEMENT;
use crate::utils::limbs_from_prev_access;
use crate::utils::pad_rows;
use core::borrow::{Borrow, BorrowMut};
use core::mem::size_of;
use num::BigUint;
use num::Zero;
use p3_air::AirBuilder;
use p3_air::{Air, BaseAir};
use p3_field::AbstractField;
use p3_field::PrimeField32;
use p3_matrix::dense::RowMajorMatrix;
use p3_matrix::MatrixRowSlices;
use sp1_derive::AlignedBorrow;
use std::fmt::Debug;
use std::marker::PhantomData;

pub const NUM_WEIERSTRASS_ADD_COLS: usize = size_of::<WeierstrassAddAssignCols<u8>>();

/// A set of columns to compute `WeierstrassAdd` that add two points on a Weierstrass curve.
///
/// Right now the number of limbs is assumed to be a constant, although this could be macro-ed or
/// made generic in the future.
#[derive(Debug, Clone, AlignedBorrow)]
#[repr(C)]
pub struct WeierstrassAddAssignCols<T> {
    pub is_real: T,
    pub shard: T,
    pub clk: T,
    pub p_ptr: T,
    pub q_ptr: T,
    pub q_ptr_access: MemoryReadCols<T>,
    pub p_access: [MemoryWriteCols<T>; NUM_WORDS_EC_POINT],
    pub q_access: [MemoryReadCols<T>; NUM_WORDS_EC_POINT],
    pub(crate) slope_denominator: FieldOpCols<T>,
    pub(crate) slope_numerator: FieldOpCols<T>,
    pub(crate) slope: FieldOpCols<T>,
    pub(crate) slope_squared: FieldOpCols<T>,
    pub(crate) p_x_plus_q_x: FieldOpCols<T>,
    pub(crate) x3_ins: FieldOpCols<T>,
    pub(crate) p_x_minus_x: FieldOpCols<T>,
    pub(crate) y3_ins: FieldOpCols<T>,
    pub(crate) slope_times_p_x_minus_x: FieldOpCols<T>,
}

#[derive(Default)]
pub struct WeierstrassAddAssignChip<E> {
    _marker: PhantomData<E>,
}

impl<E: EllipticCurve> Syscall for WeierstrassAddAssignChip<E> {
    fn execute(&self, rt: &mut SyscallContext) -> u32 {
        let event = create_ec_add_event::<E>(rt);
        rt.record_mut().weierstrass_add_events.push(event.clone());
        event.p_ptr + 1
    }

    fn num_extra_cycles(&self) -> u32 {
        8
    }
}

impl<E: EllipticCurve> WeierstrassAddAssignChip<E> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    fn populate_field_ops<F: PrimeField32>(
        cols: &mut WeierstrassAddAssignCols<F>,
        p_x: BigUint,
        p_y: BigUint,
        q_x: BigUint,
        q_y: BigUint,
    ) {
        // This populates necessary field operations to calculate the addition of two points on a
        // Weierstrass curve.

        // slope = (q.y - p.y) / (q.x - p.x).
        let slope = {
            let slope_numerator =
                cols.slope_numerator
                    .populate::<E::BaseField>(&q_y, &p_y, FieldOperation::Sub);

            let slope_denominator =
                cols.slope_denominator
                    .populate::<E::BaseField>(&q_x, &p_x, FieldOperation::Sub);

            cols.slope.populate::<E::BaseField>(
                &slope_numerator,
                &slope_denominator,
                FieldOperation::Div,
            )
        };

        // x = slope * slope - (p.x + q.x).
        let x = {
            let slope_squared =
                cols.slope_squared
                    .populate::<E::BaseField>(&slope, &slope, FieldOperation::Mul);
            let p_x_plus_q_x =
                cols.p_x_plus_q_x
                    .populate::<E::BaseField>(&p_x, &q_x, FieldOperation::Add);
            cols.x3_ins
                .populate::<E::BaseField>(&slope_squared, &p_x_plus_q_x, FieldOperation::Sub)
        };

        // y = slope * (p.x - x_3n) - p.y.
        {
            let p_x_minus_x =
                cols.p_x_minus_x
                    .populate::<E::BaseField>(&p_x, &x, FieldOperation::Sub);
            let slope_times_p_x_minus_x = cols.slope_times_p_x_minus_x.populate::<E::BaseField>(
                &slope,
                &p_x_minus_x,
                FieldOperation::Mul,
            );
            cols.y3_ins.populate::<E::BaseField>(
                &slope_times_p_x_minus_x,
                &p_y,
                FieldOperation::Sub,
            );
        }
    }
}

impl<F: PrimeField32, E: EllipticCurve + WeierstrassParameters> MachineAir<F>
    for WeierstrassAddAssignChip<E>
{
    fn name(&self) -> String {
        "WeierstrassAddAssign".to_string()
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let mut rows = Vec::new();

        let mut new_field_events = Vec::new();

        for i in 0..input.weierstrass_add_events.len() {
            let event = input.weierstrass_add_events[i].clone();
            let mut row = [F::zero(); NUM_WEIERSTRASS_ADD_COLS];
            let cols: &mut WeierstrassAddAssignCols<F> = row.as_mut_slice().borrow_mut();

            // Decode affine points.
            let p = &event.p;
            let q = &event.q;
            let p = AffinePoint::<E>::from_words_le(p);
            let (p_x, p_y) = (p.x, p.y);
            let q = AffinePoint::<E>::from_words_le(q);
            let (q_x, q_y) = (q.x, q.y);

            // Populate basic columns.
            cols.is_real = F::one();
            cols.shard = F::from_canonical_u32(event.shard);
            cols.clk = F::from_canonical_u32(event.clk);
            cols.p_ptr = F::from_canonical_u32(event.p_ptr);
            cols.q_ptr = F::from_canonical_u32(event.q_ptr);

            Self::populate_field_ops(cols, p_x, p_y, q_x, q_y);

            // Populate the memory access columns.
            for i in 0..NUM_WORDS_EC_POINT {
                cols.q_access[i].populate(event.q_memory_records[i], &mut new_field_events);
            }
            for i in 0..NUM_WORDS_EC_POINT {
                cols.p_access[i].populate(event.p_memory_records[i], &mut new_field_events);
            }
            cols.q_ptr_access
                .populate(event.q_ptr_record, &mut new_field_events);

            rows.push(row);
        }
        output.add_field_events(&new_field_events);

        pad_rows(&mut rows, || {
            let mut row = [F::zero(); NUM_WEIERSTRASS_ADD_COLS];
            let cols: &mut WeierstrassAddAssignCols<F> = row.as_mut_slice().borrow_mut();
            let zero = BigUint::zero();
            Self::populate_field_ops(cols, zero.clone(), zero.clone(), zero.clone(), zero);
            row
        });

        // Convert the trace to a row major matrix.
        RowMajorMatrix::new(
            rows.into_iter().flatten().collect::<Vec<_>>(),
            NUM_WEIERSTRASS_ADD_COLS,
        )
    }
}

impl<F, E: EllipticCurve> BaseAir<F> for WeierstrassAddAssignChip<E> {
    fn width(&self) -> usize {
        NUM_WEIERSTRASS_ADD_COLS
    }
}

impl<AB, E: EllipticCurve> Air<AB> for WeierstrassAddAssignChip<E>
where
    AB: SP1AirBuilder,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let row: &WeierstrassAddAssignCols<AB::Var> = main.row_slice(0).borrow();

        let p_x = limbs_from_prev_access(&row.p_access[0..NUM_WORDS_FIELD_ELEMENT]);
        let p_y = limbs_from_prev_access(&row.p_access[NUM_WORDS_FIELD_ELEMENT..]);

        let q_x = limbs_from_prev_access(&row.q_access[0..NUM_WORDS_FIELD_ELEMENT]);
        let q_y = limbs_from_prev_access(&row.q_access[NUM_WORDS_FIELD_ELEMENT..]);

        // slope = (q.y - p.y) / (q.x - p.x).
        let slope = {
            row.slope_numerator.eval::<AB, E::BaseField, _, _>(
                builder,
                &q_y,
                &p_y,
                FieldOperation::Sub,
            );

            row.slope_denominator.eval::<AB, E::BaseField, _, _>(
                builder,
                &q_x,
                &p_x,
                FieldOperation::Sub,
            );

            row.slope.eval::<AB, E::BaseField, _, _>(
                builder,
                &row.slope_numerator.result,
                &row.slope_denominator.result,
                FieldOperation::Div,
            );

            row.slope.result
        };

        // x = slope * slope - self.x - other.x.
        let x = {
            row.slope_squared.eval::<AB, E::BaseField, _, _>(
                builder,
                &slope,
                &slope,
                FieldOperation::Mul,
            );

            row.p_x_plus_q_x.eval::<AB, E::BaseField, _, _>(
                builder,
                &p_x,
                &q_x,
                FieldOperation::Add,
            );

            row.x3_ins.eval::<AB, E::BaseField, _, _>(
                builder,
                &row.slope_squared.result,
                &row.p_x_plus_q_x.result,
                FieldOperation::Sub,
            );

            row.x3_ins.result
        };

        // y = slope * (p.x - x_3n) - q.y.
        {
            row.p_x_minus_x
                .eval::<AB, E::BaseField, _, _>(builder, &p_x, &x, FieldOperation::Sub);

            row.slope_times_p_x_minus_x.eval::<AB, E::BaseField, _, _>(
                builder,
                &slope,
                &row.p_x_minus_x.result,
                FieldOperation::Mul,
            );

            row.y3_ins.eval::<AB, E::BaseField, _, _>(
                builder,
                &row.slope_times_p_x_minus_x.result,
                &p_y,
                FieldOperation::Sub,
            );
        }

        // Constraint self.p_access.value = [self.x3_ins.result, self.y3_ins.result]. This is to
        // ensure that p_access is updated with the new value.
        for i in 0..NUM_LIMBS {
            builder
                .when(row.is_real)
                .assert_eq(row.x3_ins.result[i], row.p_access[i / 4].value()[i % 4]);
            builder
                .when(row.is_real)
                .assert_eq(row.y3_ins.result[i], row.p_access[8 + i / 4].value()[i % 4]);
        }

        builder.constraint_memory_access(
            row.shard,
            row.clk, // clk + 0 -> C
            AB::F::from_canonical_u32(Register::X11 as u32),
            &row.q_ptr_access,
            row.is_real,
        );
        builder.constraint_memory_access_slice(
            row.shard,
            row.clk.into(), // clk + 0 -> Memory
            row.q_ptr,
            &row.q_access,
            row.is_real,
        );
        builder.constraint_memory_access_slice(
            row.shard,
            row.clk + AB::F::from_canonical_u32(4), // clk + 4 -> Memory
            row.p_ptr,
            &row.p_access,
            row.is_real,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        runtime::Program,
        utils::{run_test, setup_logger, tests::SECP256K1_ADD_ELF},
    };

    #[test]
    fn test_secp256k1_add_simple() {
        setup_logger();
        let program = Program::from(SECP256K1_ADD_ELF);
        run_test(program).unwrap();
    }
}
