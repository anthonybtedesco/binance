use eframe::egui;
use std::env;
use std::sync::Arc;
use crate::ws_client::WsClient;

pub struct BinanceApp {
    api_key: String,
    secret_key: String,
    endpoint: String,
    ws_endpoint: String,
    ws_client: Arc<WsClient>,
}

impl BinanceApp {
    pub fn new(ws_client: Arc<WsClient>) -> Self {
        Self {
            api_key: env::var("API_KEY").unwrap_or_default(),
            secret_key: env::var("SECRET_KEY").unwrap_or_default(),
            endpoint: env::var("BINANCE_API_ENDPOINT").unwrap_or_default(),
            ws_endpoint: env::var("BINANCE_WS_ENDPOINT").unwrap_or_default(),
            ws_client,
        }
    }
}

impl eframe::App for BinanceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Binance Trading Interface");
            
            ui.horizontal(|ui| {
                ui.label("WebSocket Status:");
                ui.label(&self.ws_client.get_status());
            });

            ui.horizontal(|ui| {
                ui.label("API Endpoint:");
                ui.label(&self.endpoint);
            });

            ui.horizontal(|ui| {
                ui.label("WebSocket Endpoint:");
                ui.label(&self.ws_endpoint);
            });

            ui.separator();

            // Asset balances section
            ui.heading("Asset Balances");
            egui::ScrollArea::vertical().show(ui, |ui| {
                tokio::task::block_in_place(|| {
                    if let Ok(balances) = futures::executor::block_on(self.ws_client.get_balances()) {
                        let mut sorted_assets: Vec<_> = balances.iter().collect();
                        sorted_assets.sort_by(|a, b| a.0.cmp(b.0));
                        
                        for (asset, balance) in sorted_assets {
                            if balance.free > 0.0 || balance.locked > 0.0 {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}: ", asset));
                                    ui.label(format!("Free: {:.8}", balance.free));
                                    if balance.locked > 0.0 {
                                        ui.label(format!("Locked: {:.8}", balance.locked));
                                    }
                                });
                            }
                        }
                    }
                });
            });

            ui.group(|ui| {
                ui.heading("Market Data");
                if ui.button("Fetch Price").clicked() {
                    
                }
            });

            ui.group(|ui| {
                ui.heading("Trading");
                if ui.button("Place Order").clicked() {
                    
                }
            });
        });
        
        // Request repaint to update WebSocket status
        ctx.request_repaint();
    }
}
