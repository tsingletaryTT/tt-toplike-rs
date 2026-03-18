//! Memory Dungeon - Roguelike visualization of memory hierarchy
//!
//! A dynamic, full-screen ASCII visualization showing memory flowing through the hardware hierarchy:
//! - DDR channels at bottom (external memory gates)
//! - L2 cache in middle (staging rooms)
//! - L1 SRAM above (fast cache vaults)
//! - Tensix cores at top (processing chambers)
//!
//! Memory particles (@◉◎•○·) spawn at DDR, flow upward through L2 → L1 → Tensix,
//! with real-time animation driven by actual telemetry (power, current, temperature).
//!
//! Inspired by roguelike dungeons (NetHack, DCSS), where every character and color
//! has meaning, and the dungeon is alive with activity.

use crate::animation::{AdaptiveBaseline, hsv_to_rgb, temp_to_hue, PARTICLE_CHARS};
use crate::backend::TelemetryBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A memory particle representing a memory operation flowing through the hierarchy
#[derive(Debug, Clone)]
pub struct MemoryParticle {
    /// X position (column)
    pub x: usize,
    /// Y position (row, 0=bottom)
    pub y: usize,
    /// Current layer (0=DDR, 1=L2, 2=L1, 3=Tensix)
    pub layer: usize,
    /// Target layer
    pub target_layer: usize,
    /// Intensity (0.0-1.0, driven by power)
    pub intensity: f32,
    /// Color hue (0-360, driven by temperature)
    pub hue: f32,
    /// Time to live (frames)
    pub ttl: u32,
}

impl MemoryParticle {
    /// Create new particle at DDR entry
    pub fn new(ddr_channel: usize, power_change: f32, temp: f32) -> Self {
        Self {
            x: ddr_channel * 6,  // Spread across channels
            y: 0,                // Start at bottom (DDR)
            layer: 0,            // DDR layer
            target_layer: 1,     // Move toward L2
            intensity: power_change.max(0.0).min(1.0),
            hue: temp_to_hue(temp),
            ttl: 60,
        }
    }

    /// Get character representing this particle
    pub fn get_char(&self) -> char {
        let idx = (self.intensity * (PARTICLE_CHARS.len() - 1) as f32) as usize;
        PARTICLE_CHARS[idx.min(PARTICLE_CHARS.len() - 1)]
    }

    /// Get color for this particle
    pub fn get_color(&self) -> Color {
        hsv_to_rgb(self.hue, 0.8, 0.6 + self.intensity * 0.4)
    }

    /// Update particle position (move upward through layers)
    pub fn update(&mut self) {
        // Move upward
        self.y += 1;

        // Advance through layers based on Y position
        if self.y > 15 && self.layer < 3 {
            self.layer = 3;  // Reached Tensix
        } else if self.y > 10 && self.layer < 2 {
            self.layer = 2;  // Reached L1
        } else if self.y > 5 && self.layer < 1 {
            self.layer = 1;  // Reached L2
        }

        self.ttl = self.ttl.saturating_sub(1);
    }

    /// Check if particle is still alive
    pub fn is_alive(&self) -> bool {
        self.ttl > 0
    }
}

/// Memory Dungeon visualization
pub struct MemoryCastle {
    /// Terminal width
    width: usize,
    /// Terminal height
    height: usize,
    /// Adaptive baseline for relative activity
    baseline: AdaptiveBaseline,
    /// Animation frame counter
    frame: u32,
    /// Active memory particles
    particles: Vec<MemoryParticle>,
    /// Maximum particles
    max_particles: usize,
}

impl MemoryCastle {
    /// Create new Memory Dungeon
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            baseline: AdaptiveBaseline::new(),
            frame: 0,
            particles: Vec::new(),
            max_particles: 200,  // More particles for full-screen animation
        }
    }

    /// Update animation state
    pub fn update<B: TelemetryBackend>(&mut self, backend: &B) {
        self.frame = self.frame.wrapping_add(1);

        // Update baseline for each device
        for (idx, device) in backend.devices().iter().enumerate() {
            if let Some(telem) = backend.telemetry(idx) {
                self.baseline.update(
                    device.index,
                    telem.power_w(),
                    telem.current_a(),
                    telem.temp_c(),
                    telem.aiclk_mhz() as f32,
                );
            }
        }

        // Spawn new particles based on activity
        for (_idx, device) in backend.devices().iter().enumerate() {
            if let Some(telem) = backend.telemetry(device.index) {
                let power_change = self.baseline.power_change(device.index, telem.power_w());
                let temp = telem.temp_c();

                // Spawn rate based on activity (higher activity = more particles)
                let spawn_chance = (power_change * 0.5).max(0.1).min(0.8);
                if self.particles.len() < self.max_particles && (self.frame % 3 == 0) && spawn_chance > 0.3 {
                    // Spawn at random DDR channel
                    let num_channels = device.architecture.memory_channels();
                    let channel = (self.frame as usize * 7 + device.index * 3) % num_channels;
                    self.particles.push(MemoryParticle::new(channel, power_change, temp));
                }
            }
        }

        // Update all particles
        for particle in &mut self.particles {
            particle.update();
        }

        // Remove dead particles
        self.particles.retain(|p| p.is_alive());
    }

    /// Render the Memory Dungeon
    pub fn render<B: TelemetryBackend>(&self, backend: &B) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        let devices = backend.devices();
        if devices.is_empty() {
            return lines;
        }

        // For now, render first device (we can extend to multi-device later)
        let device = &devices[0];
        let telem = backend.telemetry(0);
        let smbus = backend.smbus_telemetry(0);

        // Get metrics
        let power = telem.map(|t| t.power_w()).unwrap_or(0.0);
        let temp = telem.map(|t| t.temp_c()).unwrap_or(0.0);
        let current = telem.map(|t| t.current_a()).unwrap_or(0.0);
        let power_change = self.baseline.power_change(device.index, power);
        let current_change = self.baseline.current_change(device.index, current);

        // Calculate layer heights
        let total_height = self.height.saturating_sub(6);  // Reserve for header/footer
        let tensix_height = (total_height as f32 * 0.25) as usize;  // 25% for Tensix
        let l1_height = (total_height as f32 * 0.25) as usize;      // 25% for L1
        let l2_height = (total_height as f32 * 0.25) as usize;      // 25% for L2
        let ddr_height = total_height.saturating_sub(tensix_height + l1_height + l2_height);  // Remaining for DDR

        // === HEADER ===
        lines.push(self.render_header(device, telem, smbus));
        lines.push(self.render_separator());

        // === TENSIX CORES (Top layer) ===
        lines.extend(self.render_tensix_layer(device, tensix_height, power_change, temp));

        // === L1 SRAM ===
        lines.extend(self.render_l1_layer(device, l1_height, power_change, temp));

        // === L2 CACHE ===
        lines.extend(self.render_l2_layer(l2_height, current_change));

        // === DDR CHANNELS (Bottom layer) ===
        lines.extend(self.render_ddr_layer(device, smbus, ddr_height, current));

        // === FOOTER ===
        lines.push(self.render_separator());
        lines.push(self.render_footer());

        lines
    }

    /// Render header with device info
    fn render_header(
        &self,
        device: &crate::models::Device,
        telem: Option<&crate::models::Telemetry>,
        smbus: Option<&crate::models::SmbusTelemetry>,
    ) -> Line<'static> {
        let mut spans = Vec::new();

        // Title
        spans.push(Span::styled(
            " 🏰 MEMORY DUNGEON ",
            Style::default()
                .fg(Color::Rgb(220, 180, 255))
                .add_modifier(Modifier::BOLD),
        ));

        spans.push(Span::raw(" │ "));

        // Device info
        spans.push(Span::styled(
            format!("Device {}: {} ", device.index, device.architecture.abbrev()),
            Style::default().fg(Color::Rgb(180, 200, 255)),
        ));

        spans.push(Span::raw("│ "));

        // Temperature
        if let Some(t) = telem {
            let temp = t.temp_c();
            let temp_color = if temp > 80.0 {
                Color::Rgb(255, 100, 100)
            } else if temp > 65.0 {
                Color::Rgb(255, 180, 100)
            } else {
                Color::Rgb(100, 220, 100)
            };
            spans.push(Span::styled(
                format!("🌡 {:.1}°C ", temp),
                Style::default().fg(temp_color),
            ));

            spans.push(Span::raw("│ "));

            // Power
            spans.push(Span::styled(
                format!("⚡ {:.1}W ", t.power_w()),
                Style::default().fg(Color::Rgb(255, 220, 100)),
            ));

            spans.push(Span::raw("│ "));

            // Current
            spans.push(Span::styled(
                format!("⚙ {:.1}A ", t.current_a()),
                Style::default().fg(Color::Rgb(100, 180, 255)),
            ));
        }

        // ARC health
        if let Some(s) = smbus {
            let healthy = s.is_arc0_healthy();
            let arc_color = if healthy {
                Color::Rgb(100, 255, 100)
            } else {
                Color::Rgb(255, 100, 100)
            };
            spans.push(Span::raw("│ ARC: "));
            spans.push(Span::styled(
                if healthy { "●" } else { "○" },
                Style::default().fg(arc_color),
            ));
        }

        // Particle count
        spans.push(Span::raw(format!(" │ Particles: {} ", self.particles.len())));

        Line::from(spans)
    }

    /// Render separator line
    fn render_separator(&self) -> Line<'static> {
        Line::from(Span::styled(
            "═".repeat(self.width.min(120)),
            Style::default().fg(Color::Rgb(100, 100, 120)),
        ))
    }

    /// Render Tensix cores layer (top)
    fn render_tensix_layer(
        &self,
        device: &crate::models::Device,
        height: usize,
        power_change: f32,
        temp: f32,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Title
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "⚡ TENSIX CORES (Compute Layer) ⚡",
                Style::default()
                    .fg(Color::Rgb(255, 220, 100))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let (grid_rows, grid_cols) = device.architecture.tensix_grid();
        let rows_to_show = height.saturating_sub(3).min(grid_rows);

        // Render cores as roguelike cells
        for row in 0..rows_to_show {
            let mut spans = vec![Span::raw("  ")];

            for col in 0..grid_cols {
                // Calculate core activity (wave pattern + power)
                let wave = ((self.frame as f32 * 0.1 + row as f32 * 0.5 + col as f32 * 0.3).sin() + 1.0) / 2.0;
                let activity = (power_change * 0.7 + wave * 0.3).max(0.0).min(1.0);

                // Choose character based on activity
                let core_char = if activity > 0.8 {
                    '▓'
                } else if activity > 0.5 {
                    '▒'
                } else if activity > 0.2 {
                    '░'
                } else {
                    '·'
                };

                // Color based on temperature
                let hue = temp_to_hue(temp);
                let color = hsv_to_rgb(hue, 0.6 + activity * 0.4, 0.5 + activity * 0.5);

                spans.push(Span::styled(
                    format!("[{}]", core_char),
                    Style::default().fg(color),
                ));
            }

            // Show particles in this layer
            let particles_here: Vec<_> = self.particles.iter()
                .filter(|p| p.layer == 3 && p.y as usize % rows_to_show == row)
                .collect();

            if !particles_here.is_empty() {
                spans.push(Span::raw("  "));
                for p in particles_here.iter().take(5) {
                    spans.push(Span::styled(
                        format!("{} ", p.get_char()),
                        Style::default().fg(p.get_color()),
                    ));
                }
            }

            lines.push(Line::from(spans));
        }

        lines
    }

    /// Render L1 SRAM layer
    fn render_l1_layer(
        &self,
        device: &crate::models::Device,
        height: usize,
        power_change: f32,
        _temp: f32,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "💎 L1 SRAM (Fast Cache Vaults) 💎",
                Style::default()
                    .fg(Color::Rgb(100, 220, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let (_, cols) = device.architecture.tensix_grid();
        let vaults_to_show = (height.saturating_sub(2)).min(3);

        for row in 0..vaults_to_show {
            let mut spans = vec![Span::raw("  ")];

            // Draw vault boxes
            for col in 0..(cols.min(8)) {
                let activity = ((self.frame as f32 * 0.15 + col as f32 * 0.4).sin() + 1.0) / 2.0 * power_change;

                // Show particles in vaults
                let has_particle = self.particles.iter()
                    .any(|p| p.layer == 2 && p.x / 6 == col && (p.y as usize + row) % 3 == row);

                let vault_color = if activity > 0.6 {
                    Color::Rgb(100, 255, 255)
                } else {
                    Color::Rgb(80, 180, 200)
                };

                if row == 0 {
                    spans.push(Span::styled(" ╔═══╗", Style::default().fg(vault_color)));
                } else if row == 1 {
                    let content = if has_particle {
                        let p = self.particles.iter().find(|p| p.layer == 2 && p.x / 6 == col).unwrap();
                        format!(" ║ {} ║", p.get_char())
                    } else {
                        " ║   ║".to_string()
                    };
                    spans.push(Span::styled(content, Style::default().fg(vault_color)));
                } else {
                    spans.push(Span::styled(" ╚═══╝", Style::default().fg(vault_color)));
                }
            }

            lines.push(Line::from(spans));
        }

        lines
    }

    /// Render L2 cache layer
    fn render_l2_layer(&self, height: usize, current_change: f32) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "📚 L2 CACHE (Staging Rooms) 📚",
                Style::default()
                    .fg(Color::Rgb(255, 220, 100))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let banks = 8;
        let _rows_per_bank = (height.saturating_sub(2)) / 2;

        for bank in 0..banks.min(4) {
            let mut top_spans = vec![Span::raw("  ")];
            let mut bot_spans = vec![Span::raw("  ")];

            let activity = ((self.frame as f32 * 0.1 + bank as f32 * 0.7).cos() + 1.0) / 2.0 * current_change;
            let bank_color = if activity > 0.5 {
                Color::Rgb(255, 200, 80)
            } else {
                Color::Rgb(180, 140, 60)
            };

            // Top border
            top_spans.push(Span::styled(" ╔════════╗", Style::default().fg(bank_color)));

            // Show particles in bank
            let particles_here: Vec<_> = self.particles.iter()
                .filter(|p| p.layer == 1 && p.x / 16 == bank)
                .take(5)
                .collect();

            let mut content = String::from(" ║ ");
            for p in &particles_here {
                content.push(p.get_char());
                content.push(' ');
            }
            content.push_str(&" ".repeat(7 - particles_here.len() * 2));
            content.push_str("║");

            bot_spans.push(Span::styled(content, Style::default().fg(bank_color)));

            lines.push(Line::from(top_spans));
            lines.push(Line::from(bot_spans));
        }

        lines
    }

    /// Render DDR channels layer (bottom)
    fn render_ddr_layer(
        &self,
        device: &crate::models::Device,
        smbus: Option<&crate::models::SmbusTelemetry>,
        _height: usize,
        current: f32,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "🚪 DDR CHANNELS (Memory Gates) 🚪",
                Style::default()
                    .fg(Color::Rgb(200, 150, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        let num_channels = device.architecture.memory_channels();

        // Gate status line
        let mut gate_spans = vec![Span::raw("  ")];
        for i in 0..num_channels {
            let trained = if let Some(s) = smbus {
                s.is_ddr_channel_trained(i)
            } else {
                true
            };

            let gate_char = if trained { '●' } else { '○' };
            let gate_color = if trained {
                Color::Rgb(150, 255, 150)
            } else {
                Color::Rgb(100, 100, 120)
            };

            gate_spans.push(Span::styled(
                format!(" ╔{}╗", gate_char),
                Style::default().fg(gate_color),
            ));
        }
        lines.push(Line::from(gate_spans));

        // Particles entering from DDR
        let mut particle_spans = vec![Span::raw("  ")];
        for i in 0..num_channels {
            let particles_here: Vec<_> = self.particles.iter()
                .filter(|p| p.layer == 0 && p.x / 6 == i)
                .take(1)
                .collect();

            if let Some(p) = particles_here.first() {
                particle_spans.push(Span::styled(
                    format!("  {}  ", p.get_char()),
                    Style::default().fg(p.get_color()),
                ));
            } else {
                particle_spans.push(Span::raw("     "));
            }
        }
        lines.push(Line::from(particle_spans));

        // Utilization bars
        let mut util_spans = vec![Span::raw("  ")];
        let normalized_current = (current / 100.0).min(1.0);
        let num_blocks = (normalized_current * 8.0) as usize;
        for i in 0..num_channels {
            let block_char = if i < num_blocks {
                PARTICLE_CHARS[i.min(PARTICLE_CHARS.len() - 1)]
            } else {
                '·'
            };
            util_spans.push(Span::styled(
                format!(" [{}] ", block_char),
                Style::default().fg(Color::Rgb(255, 180, 100)),
            ));
        }
        lines.push(Line::from(util_spans));

        lines
    }

    /// Render footer with legend
    fn render_footer(&self) -> Line<'static> {
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Legend: ", Style::default().fg(Color::Rgb(150, 150, 150))),
            Span::styled("@◉◎•○· ", Style::default().fg(Color::Rgb(255, 180, 100))),
            Span::raw("= Particles (hot→cold) │ "),
            Span::styled("● ", Style::default().fg(Color::Rgb(100, 255, 100))),
            Span::raw("= Trained │ "),
            Span::styled("○ ", Style::default().fg(Color::Rgb(100, 100, 120))),
            Span::raw("= Idle │ Press "),
            Span::styled("'v'", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" to cycle views"),
        ])
    }
}
