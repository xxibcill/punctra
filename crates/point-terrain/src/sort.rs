use std::cmp::Ordering;

pub(crate) enum MergeSortError<E> {
    ScratchLength,
    Step(E),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum MergeSortOutput {
    Values,
    Scratch,
}

pub(crate) fn merge_sort_by<T: Copy, E>(
    values: &mut [T],
    scratch: &mut [T],
    mut compare: impl FnMut(T, T) -> Ordering,
    mut step: impl FnMut() -> Result<(), E>,
) -> Result<MergeSortOutput, MergeSortError<E>> {
    if values.len() != scratch.len() {
        return Err(MergeSortError::ScratchLength);
    }
    let length = values.len();
    let mut width = 1_usize;
    let mut source_is_values = true;

    while width < length {
        let (source, target) = if source_is_values {
            (&values[..], &mut scratch[..])
        } else {
            (&scratch[..], &mut values[..])
        };
        merge_pass(source, target, width, &mut compare, &mut step)?;
        source_is_values = !source_is_values;
        width = width.saturating_mul(2);
    }

    Ok(if source_is_values {
        MergeSortOutput::Values
    } else {
        MergeSortOutput::Scratch
    })
}

fn merge_pass<T: Copy, E>(
    source: &[T],
    target: &mut [T],
    width: usize,
    compare: &mut impl FnMut(T, T) -> Ordering,
    step: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), MergeSortError<E>> {
    let mut start = 0_usize;
    while start < source.len() {
        let middle = start.saturating_add(width).min(source.len());
        let end = middle.saturating_add(width).min(source.len());
        merge_run(
            source,
            &mut target[start..end],
            start,
            middle,
            end,
            compare,
            step,
        )?;
        start = end;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn merge_run<T: Copy, E>(
    source: &[T],
    target: &mut [T],
    start: usize,
    middle: usize,
    end: usize,
    compare: &mut impl FnMut(T, T) -> Ordering,
    step: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), MergeSortError<E>> {
    let mut left = start;
    let mut right = middle;
    for output in target {
        step().map_err(MergeSortError::Step)?;
        if right >= end
            || (left < middle && compare(source[left], source[right]) != Ordering::Greater)
        {
            *output = source[left];
            left += 1;
        } else {
            *output = source[right];
            right += 1;
        }
    }
    Ok(())
}
