#![expect(
    clippy::expect_used,
    reason = "benchmark setup is not a `#[test]` function, so clippy.toml's in-tests allowances do not reach it"
)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use node_agent::dsp::convert::{index_to_f64, narrow};
use node_agent::dsp::fir::Decimator;
use node_agent::dsp::fm::Discriminator;
use node_agent::dsp::metrics::ChannelSpec;
use node_agent::dsp::welch::Welch;
use node_agent::dsp::window::Window;
use node_agent::dsp::{Psd, derive};
use num_complex::Complex32;
use protocol::v1::Modulation;

const SAMPLE_RATE: u32 = 2_400_000;

const BASEBAND_RATE: u32 = 240_000;

const BLOCK: usize = 0x0001_0000;

const FFT_SIZES: [usize; 2] = [0x2000, 0x8000];

const TAPS: usize = 101;

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;

        let bits = usize::try_from(self.0 >> 11).unwrap_or(0);

        index_to_f64(bits) / index_to_f64(1_usize << 53) - 0.5
    }
}

fn signal(count: usize, sample_rate: u32) -> Vec<Complex32> {
    let mut noise = Noise(0x2545_F491_4F6C_DD1D);
    let tone_step = std::f64::consts::TAU * 1_000.0 / f64::from(sample_rate);
    let index = 60_000.0 / 1_000.0;

    (0..count)
        .map(|sample| {
            let phase = index * (tone_step * index_to_f64(sample)).sin();

            Complex32::new(
                narrow(0.01_f64.mul_add(noise.next(), 0.5 * phase.cos())),
                narrow(0.01_f64.mul_add(noise.next(), 0.5 * phase.sin())),
            )
        })
        .collect()
}

fn spectrum(fft_size: usize) -> Psd {
    let mut welch = Welch::new(fft_size, SAMPLE_RATE, Window::Hann).expect("a power of two");
    welch.push(&signal(fft_size.saturating_mul(8), SAMPLE_RATE));

    welch.finish().expect("segments were pushed")
}

fn welch(c: &mut Criterion) {
    let block = signal(BLOCK, SAMPLE_RATE);
    let mut group = c.benchmark_group("welch");
    group.throughput(Throughput::Elements(u64::try_from(BLOCK).unwrap_or(0)));

    for fft_size in FFT_SIZES {
        group.bench_with_input(
            BenchmarkId::from_parameter(fft_size),
            &fft_size,
            |bencher, &size| {
                let mut welch =
                    Welch::new(size, SAMPLE_RATE, Window::Hann).expect("a power of two");

                bencher.iter(|| {
                    welch.push(std::hint::black_box(&block));
                });
            },
        );
    }

    group.finish();
}

fn metrics(c: &mut Criterion) {
    let spec = ChannelSpec {
        frequency_hz: 89_700_000,
        modulation: Modulation::Fm,
        bandwidth_hz: 0,
    };

    let mut group = c.benchmark_group("metrics");
    for fft_size in FFT_SIZES {
        let psd = spectrum(fft_size);
        group.throughput(Throughput::Elements(u64::try_from(fft_size).unwrap_or(0)));
        group.bench_with_input(
            BenchmarkId::from_parameter(fft_size),
            &psd,
            |bencher, psd| {
                bencher.iter(|| derive(std::hint::black_box(psd), &spec));
            },
        );
    }

    group.finish();
}

fn fir(c: &mut Criterion) {
    let block = signal(BLOCK, SAMPLE_RATE);

    let mut group = c.benchmark_group("fir");
    group.throughput(Throughput::Elements(u64::try_from(BLOCK).unwrap_or(0)));
    group.bench_function("decimate_by_10", |bencher| {
        let mut decimator = Decimator::low_pass(100_000.0, SAMPLE_RATE, 10, TAPS);
        let mut out = Vec::with_capacity(BLOCK.checked_div(10).unwrap_or(0));

        bencher.iter(|| {
            out.clear();
            decimator.process(std::hint::black_box(&block), &mut out);
        });
    });

    group.finish();
}

fn discriminator(c: &mut Criterion) {
    let baseband = signal(usize::try_from(BASEBAND_RATE).unwrap_or(0), BASEBAND_RATE);

    let mut group = c.benchmark_group("fm");
    group.throughput(Throughput::Elements(u64::from(BASEBAND_RATE)));
    group.bench_function("discriminate", |bencher| {
        let mut discriminator = Discriminator::new(BASEBAND_RATE);
        let mut out = Vec::with_capacity(usize::try_from(BASEBAND_RATE).unwrap_or(0));

        bencher.iter(|| {
            out.clear();
            discriminator.process(std::hint::black_box(&baseband), &mut out);
        });
    });

    group.finish();
}

criterion_group!(benches, welch, metrics, fir, discriminator);
criterion_main!(benches);
