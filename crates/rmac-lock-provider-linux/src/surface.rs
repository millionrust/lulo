//! Bounded lock-surface configure and shared-memory buffer lifecycle.
//!
//! This state is protocol-facing but platform-neutral so its ordering rules can
//! be executed on the macOS development host. It does not allocate or paint a
//! buffer. A Linux adapter consumes each [`RenderPlan`], paints exactly the
//! requested layout, then obtains an [`Commit`] that orders `ack_configure`
//! before attach/damage/commit.

use std::collections::{btree_map::Entry, BTreeMap};
use std::fmt;

use rmac_lock_provider::OutputId;

const BYTES_PER_PIXEL: u32 = 4;
pub const MAX_SCALE: u32 = 8;
pub const MAX_BUFFER_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TOTAL_BUFFER_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_IN_FLIGHT_BUFFERS: usize = 3;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct BufferId(u64);

impl fmt::Debug for BufferId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BufferId(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferLayout {
    width: u32,
    height: u32,
    stride: u32,
    byte_len: u64,
    scale: u32,
}

impl BufferLayout {
    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    pub fn stride(self) -> u32 {
        self.stride
    }

    pub fn byte_len(self) -> u64 {
        self.byte_len
    }

    pub fn scale(self) -> u32 {
        self.scale
    }
}

pub struct RenderPlan {
    output: OutputId,
    buffer: BufferId,
    revision: u64,
    layout: BufferLayout,
}

impl RenderPlan {
    pub fn output(&self) -> OutputId {
        self.output
    }

    pub fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub fn layout(&self) -> BufferLayout {
        self.layout
    }
}

impl fmt::Debug for RenderPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderPlan")
            .field("output", &"<redacted>")
            .field("buffer", &"<redacted>")
            .field("layout", &self.layout)
            .finish()
    }
}

/// Exact protocol operations for one painted buffer.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Commit {
    output: OutputId,
    buffer: BufferId,
    ack_serial: Option<u32>,
    layout: BufferLayout,
}

impl Commit {
    pub fn output(self) -> OutputId {
        self.output
    }

    pub fn buffer(self) -> BufferId {
        self.buffer
    }

    /// If present, send `ack_configure(serial)` before attaching this buffer.
    pub fn ack_serial(self) -> Option<u32> {
        self.ack_serial
    }

    pub fn layout(self) -> BufferLayout {
        self.layout
    }
}

impl fmt::Debug for Commit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Commit")
            .field("output", &"<redacted>")
            .field("buffer", &"<redacted>")
            .field("ack_configure", &self.ack_serial.is_some())
            .field("layout", &self.layout)
            .finish()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct OutputRemoval {
    output: OutputId,
    abandoned_render: Option<BufferId>,
}

impl OutputRemoval {
    pub fn output(self) -> OutputId {
        self.output
    }

    pub fn abandoned_render(self) -> Option<BufferId> {
        self.abandoned_render
    }
}

impl fmt::Debug for OutputRemoval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputRemoval")
            .field("output", &"<redacted>")
            .field(
                "abandoned_render",
                &self.abandoned_render.map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub struct SurfaceSet {
    outputs: BTreeMap<OutputId, Surface>,
    buffers: BTreeMap<BufferId, BufferRecord>,
    next_buffer: u64,
}

impl Default for SurfaceSet {
    fn default() -> Self {
        Self {
            outputs: BTreeMap::new(),
            buffers: BTreeMap::new(),
            next_buffer: 1,
        }
    }
}

impl SurfaceSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    pub fn in_flight_buffer_count(&self) -> usize {
        self.buffers.len()
    }

    pub fn reserved_byte_count(&self) -> u64 {
        self.buffers
            .values()
            .map(|buffer| buffer.byte_len)
            .chain(
                self.outputs
                    .values()
                    .filter_map(|surface| surface.active_render.map(|render| render.byte_len)),
            )
            .sum()
    }

    /// Register an output before creating its `wl_surface` and lock role.
    pub fn add_output(&mut self, output: OutputId) -> Result<(), Error> {
        match self.outputs.entry(output) {
            Entry::Vacant(entry) => {
                entry.insert(Surface::default());
                Ok(())
            }
            Entry::Occupied(_) => Err(Error::DuplicateOutput),
        }
    }

    /// Forget the output so the adapter can destroy its role and `wl_surface`
    /// immediately. Committed buffers remain tracked until compositor release.
    pub fn remove_output(&mut self, output: OutputId) -> Result<OutputRemoval, Error> {
        let surface = self.outputs.remove(&output).ok_or(Error::UnknownOutput)?;
        Ok(OutputRemoval {
            output,
            abandoned_render: surface.active_render.map(|active| active.buffer),
        })
    }

    /// Record the newest configure. Multiple unhandled configures coalesce;
    /// only the last is acknowledged and rendered.
    pub fn configure(
        &mut self,
        output: OutputId,
        serial: u32,
        width: u32,
        height: u32,
    ) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions);
        }

        let surface = self.outputs.get_mut(&output).ok_or(Error::UnknownOutput)?;
        if surface
            .configuration
            .is_some_and(|configuration| configuration.serial == serial)
        {
            return Err(Error::DuplicateConfigure);
        }
        surface.advance_revision()?;
        surface.configuration = Some(Configuration {
            serial,
            width,
            height,
            acknowledged: false,
        });
        surface.active_render = None;
        Ok(())
    }

    /// Change the integer buffer scale. A committed surface is repainted even
    /// without a new lock-surface configure, but no serial is acknowledged.
    pub fn set_scale(&mut self, output: OutputId, scale: u32) -> Result<bool, Error> {
        if !(1..=MAX_SCALE).contains(&scale) {
            return Err(Error::InvalidScale);
        }
        let surface = self.outputs.get_mut(&output).ok_or(Error::UnknownOutput)?;
        if surface.scale == scale {
            return Ok(false);
        }
        surface.advance_revision()?;
        surface.scale = scale;
        surface.active_render = None;
        Ok(true)
    }

    /// Reserve one bounded buffer token and return its exact required layout.
    pub fn begin_render(&mut self, output: OutputId) -> Result<RenderPlan, Error> {
        let in_flight = self
            .buffers
            .values()
            .filter(|candidate| candidate.output == output)
            .count();
        if in_flight >= MAX_IN_FLIGHT_BUFFERS {
            return Err(Error::BufferBackpressure);
        }
        if self.next_buffer == 0 {
            return Err(Error::BufferIdExhausted);
        }

        let (configuration, scale, revision) = {
            let surface = self.outputs.get(&output).ok_or(Error::UnknownOutput)?;
            (
                surface.configuration.ok_or(Error::AwaitingConfigure)?,
                surface.scale,
                surface.revision,
            )
        };
        let layout = BufferLayout::new(configuration.width, configuration.height, scale)?;
        let reserved_without_current_paint = self
            .buffers
            .values()
            .map(|buffer| buffer.byte_len)
            .chain(self.outputs.iter().filter_map(|(candidate, surface)| {
                if *candidate == output {
                    None
                } else {
                    surface.active_render.map(|render| render.byte_len)
                }
            }))
            .try_fold(0_u64, u64::checked_add)
            .ok_or(Error::GlobalBufferBudget)?;
        let Some(reserved_with_paint) = reserved_without_current_paint.checked_add(layout.byte_len)
        else {
            return Err(Error::GlobalBufferBudget);
        };
        if reserved_with_paint > MAX_TOTAL_BUFFER_BYTES {
            return Err(Error::GlobalBufferBudget);
        }
        let buffer = BufferId(self.next_buffer);
        self.next_buffer = self.next_buffer.checked_add(1).unwrap_or_default();
        let surface = self.outputs.get_mut(&output).ok_or(Error::UnknownOutput)?;
        if surface.revision != revision {
            return Err(Error::StaleRender);
        }
        surface.active_render = Some(ActiveRender {
            buffer,
            revision,
            byte_len: layout.byte_len,
        });

        Ok(RenderPlan {
            output,
            buffer,
            revision,
            layout,
        })
    }

    /// Validate that no newer configure/scale superseded the paint, then
    /// produce the ordered protocol commit description.
    pub fn commit_render(&mut self, plan: RenderPlan) -> Result<Commit, Error> {
        let surface = self
            .outputs
            .get_mut(&plan.output)
            .ok_or(Error::StaleRender)?;
        let active = surface.active_render.ok_or(Error::StaleRender)?;
        if active.buffer != plan.buffer
            || active.revision != plan.revision
            || surface.revision != plan.revision
        {
            return Err(Error::StaleRender);
        }

        let configuration = surface
            .configuration
            .as_mut()
            .ok_or(Error::AwaitingConfigure)?;
        let ack_serial = (!configuration.acknowledged).then_some(configuration.serial);
        configuration.acknowledged = true;
        surface.active_render = None;
        self.buffers.insert(
            plan.buffer,
            BufferRecord {
                output: plan.output,
                byte_len: plan.layout.byte_len,
            },
        );

        Ok(Commit {
            output: plan.output,
            buffer: plan.buffer,
            ack_serial,
            layout: plan.layout,
        })
    }

    /// Destroy or recycle a `wl_buffer` only after its release event.
    pub fn release_buffer(&mut self, buffer: BufferId) -> Result<OutputId, Error> {
        self.buffers
            .remove(&buffer)
            .map(|record| record.output)
            .ok_or(Error::UnknownBuffer)
    }
}

impl fmt::Debug for SurfaceSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SurfaceSet")
            .field(
                "outputs",
                &format_args!("<{} redacted>", self.outputs.len()),
            )
            .field(
                "buffers",
                &format_args!("<{} redacted>", self.buffers.len()),
            )
            .finish()
    }
}

#[derive(Clone, Copy)]
struct Configuration {
    serial: u32,
    width: u32,
    height: u32,
    acknowledged: bool,
}

#[derive(Clone, Copy)]
struct ActiveRender {
    buffer: BufferId,
    revision: u64,
    byte_len: u64,
}

#[derive(Clone, Copy)]
struct BufferRecord {
    output: OutputId,
    byte_len: u64,
}

struct Surface {
    configuration: Option<Configuration>,
    scale: u32,
    revision: u64,
    active_render: Option<ActiveRender>,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            configuration: None,
            scale: 1,
            revision: 1,
            active_render: None,
        }
    }
}

impl Surface {
    fn advance_revision(&mut self) -> Result<(), Error> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?;
        Ok(())
    }
}

impl BufferLayout {
    fn new(logical_width: u32, logical_height: u32, scale: u32) -> Result<Self, Error> {
        let width = logical_width
            .checked_mul(scale)
            .ok_or(Error::BufferTooLarge)?;
        let height = logical_height
            .checked_mul(scale)
            .ok_or(Error::BufferTooLarge)?;
        let stride = width
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(Error::BufferTooLarge)?;
        let byte_len = u64::from(stride)
            .checked_mul(u64::from(height))
            .ok_or(Error::BufferTooLarge)?;
        if width > i32::MAX as u32
            || height > i32::MAX as u32
            || stride > i32::MAX as u32
            || byte_len > MAX_BUFFER_BYTES
        {
            return Err(Error::BufferTooLarge);
        }
        Ok(Self {
            width,
            height,
            stride,
            byte_len,
            scale,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    DuplicateOutput,
    UnknownOutput,
    AwaitingConfigure,
    DuplicateConfigure,
    InvalidDimensions,
    InvalidScale,
    BufferTooLarge,
    GlobalBufferBudget,
    BufferBackpressure,
    StaleRender,
    UnknownBuffer,
    RevisionExhausted,
    BufferIdExhausted,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "lock-surface transition failed ({self:?})")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(value: u64) -> OutputId {
        OutputId::new(value).unwrap()
    }

    fn configured(set: &mut SurfaceSet, output: OutputId) {
        set.add_output(output).unwrap();
        set.configure(output, 41, 1440, 900).unwrap();
    }

    #[test]
    fn first_commit_acks_the_exact_configure_and_layout() {
        let id = output(1);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        set.set_scale(id, 2).unwrap();

        let plan = set.begin_render(id).unwrap();
        assert_eq!(plan.layout().width(), 2880);
        assert_eq!(plan.layout().height(), 1800);
        assert_eq!(plan.layout().stride(), 11_520);
        assert_eq!(plan.layout().byte_len(), 20_736_000);
        let commit = set.commit_render(plan).unwrap();
        assert_eq!(commit.ack_serial(), Some(41));
        assert_eq!(commit.layout().scale(), 2);
        assert_eq!(set.in_flight_buffer_count(), 1);
    }

    #[test]
    fn multiple_configures_coalesce_and_stale_render_cannot_commit() {
        let id = output(1);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        let stale = set.begin_render(id).unwrap();
        set.configure(id, 42, 1280, 720).unwrap();
        assert_eq!(set.commit_render(stale), Err(Error::StaleRender));

        set.configure(id, 43, 1920, 1080).unwrap();
        let plan = set.begin_render(id).unwrap();
        let commit = set.commit_render(plan).unwrap();
        assert_eq!(commit.ack_serial(), Some(43));
        assert_eq!(commit.layout().width(), 1920);
    }

    #[test]
    fn scale_only_repaint_does_not_ack_a_serial_twice() {
        let id = output(1);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        let plan = set.begin_render(id).unwrap();
        let first = set.commit_render(plan).unwrap();
        assert_eq!(first.ack_serial(), Some(41));

        assert!(set.set_scale(id, 2).unwrap());
        let plan = set.begin_render(id).unwrap();
        let scaled = set.commit_render(plan).unwrap();
        assert_eq!(scaled.ack_serial(), None);
        assert_eq!(scaled.layout().width(), 2880);
    }

    #[test]
    fn output_removal_abandons_paint_but_waits_for_buffer_release() {
        let id = output(7);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        let plan = set.begin_render(id).unwrap();
        let committed = set.commit_render(plan).unwrap();
        let abandoned = set.begin_render(id).unwrap();

        let removal = set.remove_output(id).unwrap();
        assert_eq!(removal.abandoned_render(), Some(abandoned.buffer()));
        assert_eq!(set.output_count(), 0);
        assert_eq!(set.in_flight_buffer_count(), 1);
        assert_eq!(set.release_buffer(committed.buffer()).unwrap(), id);
        assert_eq!(set.in_flight_buffer_count(), 0);
    }

    #[test]
    fn buffer_backpressure_is_bounded_until_release() {
        let id = output(1);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        let mut buffers = Vec::new();
        for _ in 0..MAX_IN_FLIGHT_BUFFERS {
            let plan = set.begin_render(id).unwrap();
            let commit = set.commit_render(plan).unwrap();
            buffers.push(commit.buffer());
        }
        assert!(matches!(
            set.begin_render(id),
            Err(Error::BufferBackpressure)
        ));
        set.release_buffer(buffers[0]).unwrap();
        assert!(set.begin_render(id).is_ok());
    }

    #[test]
    fn malformed_or_excessive_dimensions_fail_before_allocation() {
        let id = output(1);
        let mut set = SurfaceSet::new();
        set.add_output(id).unwrap();
        assert_eq!(set.configure(id, 1, 0, 900), Err(Error::InvalidDimensions));
        set.configure(id, 2, u32::MAX, u32::MAX).unwrap();
        assert!(matches!(set.begin_render(id), Err(Error::BufferTooLarge)));
        assert_eq!(set.in_flight_buffer_count(), 0);
    }

    #[test]
    fn aggregate_active_paints_cannot_exceed_the_global_budget() {
        let mut set = SurfaceSet::new();
        for value in 1..=4 {
            let id = output(value);
            set.add_output(id).unwrap();
            set.configure(id, value as u32, 8192, 8192).unwrap();
            set.begin_render(id).unwrap();
        }
        assert_eq!(set.reserved_byte_count(), MAX_TOTAL_BUFFER_BYTES);

        let fifth = output(5);
        set.add_output(fifth).unwrap();
        set.configure(fifth, 5, 8192, 8192).unwrap();
        assert!(matches!(
            set.begin_render(fifth),
            Err(Error::GlobalBufferBudget)
        ));
    }

    #[test]
    fn output_buffer_and_plan_diagnostics_are_redacted() {
        let id = output(8_675_309);
        let mut set = SurfaceSet::new();
        configured(&mut set, id);
        let plan = set.begin_render(id).unwrap();
        let debug = format!("{set:?} {plan:?} {:?}", plan.buffer());
        assert!(!debug.contains("8675309"));
        assert!(debug.contains("<redacted>"));
    }
}
