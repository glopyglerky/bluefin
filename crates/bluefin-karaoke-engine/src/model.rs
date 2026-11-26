use crate::{Cancellation, Error, checkpoint::Checkpoint};
use candle_core::{Device, Tensor};

struct Conv2d {
    weight: Tensor,
    bias: Tensor,
    stride: usize,
    dilation: usize,
}
impl Conv2d {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let x = x.conv2d(&self.weight, 0, self.stride, self.dilation, 1)?;
        x.broadcast_add(&self.bias.reshape((1, self.bias.elem_count(), 1, 1))?)
    }
}
struct ConvTranspose2d {
    weight: Tensor,
    bias: Tensor,
}
impl ConvTranspose2d {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let x = x.conv_transpose2d(&self.weight, 0, 0, 2, 1)?;
        x.broadcast_add(&self.bias.reshape((1, self.bias.elem_count(), 1, 1))?)
    }
}

pub const FRAMES: usize = 512;
pub const BINS: usize = 1024;
const CHANNELS: [usize; 7] = [2, 16, 32, 64, 128, 256, 512];

struct BatchNorm {
    scale: Tensor,
    offset: Tensor,
}
impl BatchNorm {
    fn load(c: &Checkpoint, index: usize, channels: usize) -> Result<Self, Error> {
        let name = layer("batch_normalization", index);
        let get = |key| tensor(c, &format!("{name}/{key}"), &[channels]);
        let gamma = get("gamma")?;
        let beta = get("beta")?;
        let mean = get("moving_mean")?;
        let variance = get("moving_variance")?;
        let scale = gamma.div(&(variance + 0.001)?.sqrt()?)?;
        let offset = beta.sub(&mean.mul(&scale)?)?;
        Ok(Self {
            scale: scale.reshape((1, channels, 1, 1))?,
            offset: offset.reshape((1, channels, 1, 1))?,
        })
    }
    fn apply(&self, x: &Tensor) -> Result<Tensor, Error> {
        Ok(x.broadcast_mul(&self.scale)?.broadcast_add(&self.offset)?)
    }
}
fn layer(prefix: &str, index: usize) -> String {
    if index == 0 {
        prefix.to_owned()
    } else {
        format!("{prefix}_{index}")
    }
}
fn tensor(c: &Checkpoint, name: &str, shape: &[usize]) -> Result<Tensor, Error> {
    let w = c.weights.get(name).ok_or(Error::InvalidModel)?;
    if w.shape != shape {
        return Err(Error::InvalidModel);
    }
    Ok(Tensor::from_slice(&w.values, shape, &Device::Cpu)?)
}
fn conv(
    c: &Checkpoint,
    index: usize,
    input: usize,
    output: usize,
    size: usize,
    stride: usize,
    dilation: usize,
) -> Result<Conv2d, Error> {
    let name = layer("conv2d", index);
    let w = tensor(c, &format!("{name}/kernel"), &[size, size, input, output])?
        .permute((3, 2, 0, 1))?
        .contiguous()?;
    let b = tensor(c, &format!("{name}/bias"), &[output])?;
    Ok(Conv2d {
        weight: w,
        bias: b,
        stride,
        dilation,
    })
}
struct Unet {
    down: Vec<Conv2d>,
    down_bn: Vec<BatchNorm>,
    up: Vec<ConvTranspose2d>,
    up_bn: Vec<BatchNorm>,
    last: Conv2d,
}
impl Unet {
    fn load(c: &Checkpoint, instrument: usize) -> Result<Self, Error> {
        let mut down = Vec::new();
        let mut down_bn = Vec::new();
        for i in 0..6 {
            down.push(conv(
                c,
                instrument * 7 + i,
                CHANNELS[i],
                CHANNELS[i + 1],
                5,
                2,
                1,
            )?);
            down_bn.push(BatchNorm::load(c, instrument * 12 + i, CHANNELS[i + 1])?);
        }
        let mut up = Vec::new();
        let mut up_bn = Vec::new();
        let ins = [512, 512, 256, 128, 64, 32];
        let outs = [256, 128, 64, 32, 16, 1];
        for i in 0..6 {
            let name = layer("conv2d_transpose", instrument * 6 + i);
            let w = tensor(c, &format!("{name}/kernel"), &[5, 5, outs[i], ins[i]])?
                .permute((3, 2, 0, 1))?
                .contiguous()?;
            let b = tensor(c, &format!("{name}/bias"), &[outs[i]])?;
            up.push(ConvTranspose2d { weight: w, bias: b });
            up_bn.push(BatchNorm::load(c, instrument * 12 + 6 + i, outs[i])?);
        }
        Ok(Self {
            down,
            down_bn,
            up,
            up_bn,
            last: conv(c, instrument * 7 + 6, 1, 2, 4, 1, 2)?,
        })
    }
    fn forward(&self, input: &Tensor, cancel: &Cancellation) -> Result<Tensor, Error> {
        let mut raw = Vec::new();
        let mut x = input.clone();
        for i in 0..6 {
            cancel.check()?;
            // TF SAME, even input, kernel 5, stride 2: before=1, after=2.
            x = self.down[i].forward(&x.pad_with_zeros(2, 1, 2)?.pad_with_zeros(3, 1, 2)?)?;
            raw.push(x.clone());
            if i < 5 {
                x = self.down_bn[i].apply(&x)?;
                x = x.maximum(&(&x * 0.2)?)?;
            } // The publisher feeds raw conv6 to the decoder, not BN/activation.
        }
        for i in 0..6 {
            cancel.check()?;
            let (_, _, h, w) = x.dims4()?;
            // Adjoint TF SAME: discard one leading and two trailing values.
            x = self.up[i]
                .forward(&x)?
                .narrow(2, 1, h * 2)?
                .narrow(3, 1, w * 2)?
                .contiguous()?;
            x = self.up_bn[i].apply(&x.relu()?)?;
            if i < 5 {
                x = Tensor::cat(&[&raw[4 - i], &x], 1)?;
            }
        }
        cancel.check()?;
        let x = self
            .last
            .forward(&x.pad_with_zeros(2, 3, 3)?.pad_with_zeros(3, 3, 3)?)?;
        Ok((x.neg()?.exp()? + 1.)?.recip()?.mul(input)?)
    }
}

/// Fixed official 2-stem U-Net on CPU. One patch at a time; no training or dropout.
pub struct Model {
    unets: [Unet; 2],
}
impl Model {
    pub fn load(c: &Checkpoint) -> Result<Self, Error> {
        Ok(Self {
            unets: [Unet::load(c, 0)?, Unet::load(c, 1)?],
        })
    }
    /// Channel-major [2,512,1024] magnitudes; returns the two estimated magnitudes.
    pub fn infer(&self, magnitudes: &[f32], cancel: &Cancellation) -> Result<[Vec<f32>; 2], Error> {
        if magnitudes.len() != 2 * FRAMES * BINS
            || magnitudes.iter().any(|x| !x.is_finite() || *x < 0.)
        {
            return Err(Error::InvalidInput);
        }
        let x = Tensor::from_slice(magnitudes, (1, 2, FRAMES, BINS), &Device::Cpu)?;
        let mut outputs = [Vec::new(), Vec::new()];
        for (i, unet) in self.unets.iter().enumerate() {
            let y = unet.forward(&x, cancel)?.flatten_all()?.to_vec1::<f32>()?;
            cancel.check()?;
            if y.len() != magnitudes.len() || y.iter().any(|x| !x.is_finite() || *x < 0.) {
                return Err(Error::InvalidOutput);
            }
            outputs[i] = y;
        }
        Ok(outputs)
    }
}
