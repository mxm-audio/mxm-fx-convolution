//! Small dependency-free radix-2 FFT used by the partitioned FIR.
//!
//! This is an original iterative Cooley-Tukey implementation. It exists here rather than in a
//! shared crate because the collection does not pre-generalise DSP from one consumer.

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    #[inline]
    pub fn multiply(self, rhs: Self) -> Self {
        Self {
            re: flush(self.re * rhs.re - self.im * rhs.im),
            im: flush(self.re * rhs.im + self.im * rhs.re),
        }
    }
}

pub(crate) fn transform(values: &mut [Complex], inverse: bool) {
    debug_assert!(values.len().is_power_of_two());
    let n = values.len();

    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            values.swap(i, j);
        }
    }

    let sign = if inverse { 1.0 } else { -1.0 };
    let mut width = 2;
    while width <= n {
        let angle = sign * core::f32::consts::TAU / width as f32;
        let step = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for start in (0..n).step_by(width) {
            let mut twiddle = Complex { re: 1.0, im: 0.0 };
            for offset in 0..width / 2 {
                let even = values[start + offset];
                let odd = values[start + offset + width / 2].multiply(twiddle);
                values[start + offset] = Complex {
                    re: flush(even.re + odd.re),
                    im: flush(even.im + odd.im),
                };
                values[start + offset + width / 2] = Complex {
                    re: flush(even.re - odd.re),
                    im: flush(even.im - odd.im),
                };
                twiddle = twiddle.multiply(step);
            }
        }
        width *= 2;
    }

    if inverse {
        let scale = 1.0 / n as f32;
        for value in values {
            value.re = flush(value.re * scale);
            value.im = flush(value.im * scale);
        }
    }
}

#[inline]
pub(crate) fn flush(value: f32) -> f32 {
    if !value.is_finite() {
        0.0
    } else if value != 0.0 && value.abs() < f32::MIN_POSITIVE {
        0.0f32.copysign(value)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_inverse_round_trip() {
        let mut values = [Complex::ZERO; 16];
        for (index, value) in values.iter_mut().enumerate() {
            value.re = (index as f32 * 0.37).sin();
        }
        let original = values;
        transform(&mut values, false);
        transform(&mut values, true);
        for (actual, expected) in values.iter().zip(original) {
            assert!((actual.re - expected.re).abs() < 2.0e-6);
            assert!(actual.im.abs() < 2.0e-6);
        }
    }

    #[test]
    fn numeric_seam_flushes_faults_and_subnormals() {
        assert_eq!(flush(f32::NAN), 0.0);
        assert_eq!(flush(f32::INFINITY), 0.0);
        assert_eq!(flush(f32::from_bits(1)).to_bits(), 0.0f32.to_bits());
        assert_eq!(flush(-f32::from_bits(1)).to_bits(), (-0.0f32).to_bits());
        assert_eq!(flush(0.25), 0.25);
    }
}
