//! Live host sampling authority for System Monitor.

use gpui::{Context, Entity};
use rmac_ui::TableState;
use sysinfo::Networks;

use crate::cpu_ticks;
use crate::host_stats;
use crate::metrics::{Aggregates, History, NetIface, REFRESH_SECS};
use crate::process_table::{resync_selection, ProcessTableDelegate};
use crate::view::MonitorView;

pub(crate) struct Sampler {
    networks: Networks,
    pub(crate) aggregates: Aggregates,
    pub(crate) history: History,
    pub(crate) interfaces: Vec<NetIface>,
    prev_cpu_ticks: Option<[u64; 4]>,
    pub(crate) cpu_split: Option<(f32, f32, f32)>,
}

impl Sampler {
    pub(crate) fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            aggregates: Aggregates::default(),
            history: History::default(),
            interfaces: Vec::new(),
            prev_cpu_ticks: None,
            cpu_split: None,
        }
    }

    pub(crate) fn refresh(
        &mut self,
        table: &Entity<TableState<ProcessTableDelegate>>,
        cx: &mut Context<MonitorView>,
    ) {
        if let Some(now) = cpu_ticks::read() {
            if let Some(previous) = self.prev_cpu_ticks {
                self.cpu_split = cpu_ticks::split(previous, now);
            }
            self.prev_cpu_ticks = Some(now);
        }

        self.networks.refresh(true);
        let (network_received, network_sent) = self
            .networks
            .list()
            .values()
            .fold((0u64, 0u64), |(received, sent), data| {
                (received + data.received(), sent + data.transmitted())
            });

        self.interfaces = self
            .networks
            .list()
            .iter()
            .map(|(name, data)| NetIface {
                name: name.clone(),
                total_recv: data.total_received(),
                total_sent: data.total_transmitted(),
                recv_rate: data.received() as f64 / REFRESH_SECS,
                sent_rate: data.transmitted() as f64 / REFRESH_SECS,
            })
            .collect();
        self.interfaces.sort_by(|left, right| {
            (right.total_recv + right.total_sent)
                .cmp(&(left.total_recv + left.total_sent))
                .then_with(|| left.name.cmp(&right.name))
        });

        let mut aggregates = Aggregates::default();
        table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.refresh();
            aggregates.energy_total = delegate.all_rows.iter().map(|row| row.energy).sum();
            aggregates.mem_used = delegate.system.used_memory();
            aggregates.mem_total = delegate.system.total_memory();
            aggregates.swap_used = delegate.system.used_swap();

            let (read, write) = delegate.system.processes().values().fold(
                (0u64, 0u64),
                |(read, write), process| {
                    let usage = process.disk_usage();
                    (read + usage.read_bytes, write + usage.written_bytes)
                },
            );
            aggregates.disk_read_rate = read as f64 / REFRESH_SECS;
            aggregates.disk_write_rate = write as f64 / REFRESH_SECS;

            resync_selection(state, cx);
            state.refresh(cx);
        });

        aggregates.net_recv_rate = network_received as f64 / REFRESH_SECS;
        aggregates.net_sent_rate = network_sent as f64 / REFRESH_SECS;
        (aggregates.net_total_recv, aggregates.net_total_sent) =
            self.interfaces
                .iter()
                .fold((0u64, 0u64), |(received, sent), interface| {
                    (
                        received.saturating_add(interface.total_recv),
                        sent.saturating_add(interface.total_sent),
                    )
                });
        aggregates.threads = host_stats::thread_count();
        aggregates.mem_cached = host_stats::cached_bytes();

        let (user, system) = self
            .cpu_split
            .map_or((0.0, 0.0), |(user, system, _)| (user, system));
        History::push(&mut self.history.cpu_user, user);
        History::push(&mut self.history.cpu_system, system);
        History::push(
            &mut self.history.disk_read,
            aggregates.disk_read_rate as f32,
        );
        History::push(
            &mut self.history.disk_write,
            aggregates.disk_write_rate as f32,
        );
        History::push(&mut self.history.net_recv, aggregates.net_recv_rate as f32);
        History::push(&mut self.history.net_sent, aggregates.net_sent_rate as f32);

        let memory_percent = if aggregates.mem_total > 0 {
            (aggregates.mem_used as f32 / aggregates.mem_total as f32) * 100.0
        } else {
            0.0
        };
        History::push(&mut self.history.mem, memory_percent);
        History::push(&mut self.history.energy, aggregates.energy_total.min(100.0));

        self.aggregates = aggregates;
    }
}
