use std::borrow::BorrowMut;

use p3_field::PrimeField;
use p3_matrix::dense::RowMajorMatrix;

use crate::{air::MachineAir, runtime::ExecutionRecord};

use super::{ShaExtendChip, ShaExtendCols, NUM_SHA_EXTEND_COLS};

impl<F: PrimeField> MachineAir<F> for ShaExtendChip {
    fn name(&self) -> String {
        "ShaExtend".to_string()
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let mut rows = Vec::new();

        let mut new_field_events = Vec::new();
        for i in 0..input.sha_extend_events.len() {
            let event = input.sha_extend_events[i].clone();
            for j in 0..48usize {
                let mut row = [F::zero(); NUM_SHA_EXTEND_COLS];
                let cols: &mut ShaExtendCols<F> = row.as_mut_slice().borrow_mut();

                cols.populate_flags(j);
                cols.shard = F::from_canonical_u32(event.shard);
                cols.clk = F::from_canonical_u32(event.clk);
                cols.w_ptr = F::from_canonical_u32(event.w_ptr);

                cols.w_i_minus_15
                    .populate(event.w_i_minus_15_reads[j], &mut new_field_events);
                cols.w_i_minus_2
                    .populate(event.w_i_minus_2_reads[j], &mut new_field_events);
                cols.w_i_minus_16
                    .populate(event.w_i_minus_16_reads[j], &mut new_field_events);
                cols.w_i_minus_7
                    .populate(event.w_i_minus_7_reads[j], &mut new_field_events);

                // Compute `s0`.
                let w_i_minus_15 = event.w_i_minus_15_reads[j].value;
                let w_i_minus_15_rr_7 = cols.w_i_minus_15_rr_7.populate(output, w_i_minus_15, 7);
                let w_i_minus_15_rr_18 = cols.w_i_minus_15_rr_18.populate(output, w_i_minus_15, 18);
                let w_i_minus_15_rs_3 = cols.w_i_minus_15_rs_3.populate(output, w_i_minus_15, 3);
                let s0_intermediate =
                    cols.s0_intermediate
                        .populate(output, w_i_minus_15_rr_7, w_i_minus_15_rr_18);
                let s0 = cols.s0.populate(output, s0_intermediate, w_i_minus_15_rs_3);

                // Compute `s1`.
                let w_i_minus_2 = event.w_i_minus_2_reads[j].value;
                let w_i_minus_2_rr_17 = cols.w_i_minus_2_rr_17.populate(output, w_i_minus_2, 17);
                let w_i_minus_2_rr_19 = cols.w_i_minus_2_rr_19.populate(output, w_i_minus_2, 19);
                let w_i_minus_2_rs_10 = cols.w_i_minus_2_rs_10.populate(output, w_i_minus_2, 10);
                let s1_intermediate =
                    cols.s1_intermediate
                        .populate(output, w_i_minus_2_rr_17, w_i_minus_2_rr_19);
                let s1 = cols.s1.populate(output, s1_intermediate, w_i_minus_2_rs_10);

                // Compute `s2`.
                let w_i_minus_7 = event.w_i_minus_7_reads[j].value;
                let w_i_minus_16 = event.w_i_minus_16_reads[j].value;
                cols.s2.populate(output, w_i_minus_16, s0, w_i_minus_7, s1);

                cols.w_i
                    .populate(event.w_i_writes[j], &mut new_field_events);

                cols.is_real = F::one();
                rows.push(row);
            }
        }

        output.add_field_events(&new_field_events);

        let nb_rows = rows.len();
        let mut padded_nb_rows = nb_rows.next_power_of_two();
        if padded_nb_rows == 2 || padded_nb_rows == 1 {
            padded_nb_rows = 4;
        }
        for i in nb_rows..padded_nb_rows {
            let mut row = [F::zero(); NUM_SHA_EXTEND_COLS];
            let cols: &mut ShaExtendCols<F> = row.as_mut_slice().borrow_mut();
            cols.populate_flags(i);
            rows.push(row);
        }

        // Convert the trace to a row major matrix.
        RowMajorMatrix::new(
            rows.into_iter().flatten().collect::<Vec<_>>(),
            NUM_SHA_EXTEND_COLS,
        )
    }
}
