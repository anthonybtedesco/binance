use eframe::egui;
use log::{debug, info};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::Mutex;
use crate::ws_client::{PriceData, SymbolInfo, WsClient};
use crate::trade::{OrderSide, OrderState, OrderTracker, OrderType, TradeOrder};

pub struct BinanceApp {
    api_key: String,
    secret_key: String,
    endpoint: String,
    ws_endpoint: String,
    ws_client: Arc<Mutex<WsClient>>,
    market_filter: String,
    filtered_symbols: Vec<String>,
    last_refresh: std::time::Instant,
    selected_symbols: HashMap<String, PriceData>,
    trade_order: TradeOrder,
    order_tracker: OrderTracker,
}

impl BinanceApp {
    pub fn new(ws_client: Arc<Mutex<WsClient>>) -> Self {
        let order_tracker = TradeOrder::new_tracker();
        let mut app = Self {
            api_key: env::var("API_KEY").unwrap_or_default(),
            secret_key: env::var("SECRET_KEY").unwrap_or_default(),
            endpoint: env::var("BINANCE_API_ENDPOINT").unwrap_or_default(),
            ws_endpoint: env::var("BINANCE_WS_ENDPOINT").unwrap_or_default(),
            ws_client: ws_client.clone(),
            market_filter: String::new(),
            filtered_symbols: Vec::new(),
            last_refresh: std::time::Instant::now(),
            selected_symbols: HashMap::new(),
            trade_order: TradeOrder {
                symbol: String::new(),
                side: OrderSide::Buy,
                order_type: OrderType::Limit,
                quantity: 0.0,
                price: None,
            },
            order_tracker: order_tracker.clone(),
        };
        if let Ok(mut client) = ws_client.lock() {
            client.set_order_tracker(order_tracker);
        }
        app
    }

    fn update_filtered_symbols(&mut self) {
        let filter = self.market_filter.to_uppercase();
        if let Ok(client) = self.ws_client.lock() {
            self.filtered_symbols = client.get_symbols().iter()
                .filter(|symbol| {
                    if filter.is_empty() {
                        true
                    } else {
                        symbol.contains(&filter)
                    }
                })
                .cloned()
                .collect();
        }
    }

    fn show_trade_modal(&mut self, ui: &mut egui::Ui, symbol: &str, price_data: &PriceData, symbol_info: &SymbolInfo) {
        egui::Window::new(format!("Trade {}", symbol))
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Market Information");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            self.selected_symbols.remove(symbol);
                        }
                    });
                });

                ui.horizontal(|ui| {
                    ui.label(format!("Current Price: {:.8}", price_data.price));
                    ui.label(format!("24h Range: {:.8} - {:.8}", price_data.low_24h, price_data.high_24h));
                });

                ui.separator();
                ui.heading("Place Order");
                
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.trade_order.side, OrderSide::Buy, "Buy");
                    ui.selectable_value(&mut self.trade_order.side, OrderSide::Sell, "Sell");
                });

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.trade_order.order_type, OrderType::Limit, "Limit");
                    ui.selectable_value(&mut self.trade_order.order_type, OrderType::Market, "Market");
                });

                if let OrderType::Limit = self.trade_order.order_type {
                    let mut price = self.trade_order.price.unwrap_or(price_data.price);
                    if ui.add(egui::DragValue::new(&mut price)
                        .speed(symbol_info.tick_size)
                        .range(symbol_info.min_price..=symbol_info.max_price)
                    ).changed() {
                        self.trade_order.price = Some(price);
                    }
                }

                let mut quantity = self.trade_order.quantity;
                if ui.add(egui::DragValue::new(&mut quantity)
                    .speed(symbol_info.step_size)
                    .range(symbol_info.min_qty..=symbol_info.max_qty)
                ).changed() {
                    self.trade_order.quantity = quantity;
                }

                if ui.button("Place Order").clicked() {
                    self.trade_order.symbol = symbol.to_string();
                    let _ = futures::executor::block_on(self.trade_order.submit(self.order_tracker.clone()));
                    self.selected_symbols.remove(symbol);
                }
            });
    }

    fn render_modals(&mut self, ui: &mut egui::Ui) {
        if let Ok(client) = self.ws_client.lock() {
            let exchange_info = client.get_exchange_info();
            let selected_symbols = self.selected_symbols.clone();

            for (symbol, price_data) in selected_symbols {
                if let Some(symbol_info) = exchange_info.get(&symbol) {
                    egui::Window::new(format!("Trade {}", symbol))
                        .open(&mut self.selected_symbols.contains_key(&symbol))
                        .show(ui.ctx(), |ui| {
                            ui.separator();
                            ui.heading("Place Order");
                            
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.trade_order.side, OrderSide::Buy, "Buy");
                                ui.selectable_value(&mut self.trade_order.side, OrderSide::Sell, "Sell");
                            });

                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut self.trade_order.order_type, OrderType::Limit, "Limit");
                                ui.selectable_value(&mut self.trade_order.order_type, OrderType::Market, "Market");
                            });

                            if let OrderType::Limit = self.trade_order.order_type {
                                let mut price = self.trade_order.price.unwrap_or(price_data.price);
                                if ui.add(egui::DragValue::new(&mut price)
                                    .speed(symbol_info.tick_size)
                                    .range(symbol_info.min_price..=symbol_info.max_price)
                                ).changed() {
                                    self.trade_order.price = Some(price);
                                }
                            }

                            let mut quantity = self.trade_order.quantity;
                            if ui.add(egui::DragValue::new(&mut quantity)
                                .speed(symbol_info.step_size)
                                .range(symbol_info.min_qty..=symbol_info.max_qty)
                            ).changed() {
                                self.trade_order.quantity = quantity;
                            }

                            if ui.button("Place Order").clicked() {
                                self.trade_order.symbol = symbol.to_string();
                                let _ = futures::executor::block_on(self.trade_order.submit(self.order_tracker.clone()));
                                self.selected_symbols.remove(&symbol);
                            }
                        });
                }
            }
        }
    }

    fn render_market_data(&mut self, ui: &mut egui::Ui) {
        ui.heading("Market Data");
        ui.horizontal(|ui| {
            ui.label("Filter:");
            if ui.add(egui::TextEdit::singleline(&mut self.market_filter)).changed() {
                self.update_filtered_symbols();
            }
            ui.label(format!("({} pairs)", self.filtered_symbols.len()));
        });

        egui::ScrollArea::vertical()
            .id_salt("market_scroll")
            .show(ui, |ui| {
                tokio::task::block_in_place(|| {
                    if let Ok(client) = self.ws_client.lock() {
                        if let Ok(prices) = futures::executor::block_on(client.get_prices()) {
                            let filtered_symbols = self.filtered_symbols.clone();
                            for symbol in &filtered_symbols {
                                if let Some(price_data) = prices.get(symbol) {
                                    if ui.horizontal(|ui| {
                                        ui.add(egui::Label::new(format!("{}: ", symbol)));
                                        let clicked = ui.button("Trade")
                                            .on_hover_text(format!("Trade {}", symbol))
                                            .clicked();
                                        ui.add(egui::Label::new(format!("Price: {:.8}", price_data.price)));
                                        ui.add(egui::Label::new(format!("24h High: {:.8}", price_data.high_24h)));
                                        ui.add(egui::Label::new(format!("24h Low: {:.8}", price_data.low_24h)));
                                        ui.add(egui::Label::new(format!("Volume: {:.2}", price_data.volume)));
                                        clicked
                                    }).inner {
                                        self.selected_symbols.insert(symbol.clone(), price_data.clone());
                                    }
                                }
                            }
                        }
                    }
                });
            });
    }

    fn render_trades_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Recent Trades");
        
        egui::ScrollArea::vertical()
            .id_salt("trades_scroll")
            .show(ui, |ui| {
                tokio::task::block_in_place(|| {
                    if let Ok(orders) = self.order_tracker.try_lock() {
                        let mut sorted_orders: Vec<_> = orders.values().collect();
                        sorted_orders.sort_by(|a, b| b.last_update.cmp(&a.last_update));

                        for order in sorted_orders {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}: ", order.symbol));
                                let side_color = match order.side {
                                    OrderSide::Buy => egui::Color32::GREEN,
                                    OrderSide::Sell => egui::Color32::RED,
                                };
                                ui.colored_label(side_color, format!("{:?}", order.side));
                                ui.label(format!("Price: {:.8}", order.price));
                                ui.label(format!("Qty: {:.8}", order.quantity));
                                ui.label(format!("Filled: {:.8}", order.filled));
                                let status_color = match order.status {
                                    OrderState::Filled => egui::Color32::GREEN,
                                    OrderState::PartiallyFilled => egui::Color32::YELLOW,
                                    OrderState::Rejected | OrderState::Canceled => egui::Color32::RED,
                                    _ => ui.style().visuals.text_color(),
                                };
                                ui.colored_label(status_color, format!("{:?}", order.status));
                                ui.label(format!("Time: {}", order.last_update.format("%H:%M:%S")));
                            });
                        }
                    }
                });
            });
    }
}

impl eframe::App for BinanceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Auto-refresh balances every 3 seconds
        if self.last_refresh.elapsed() >= std::time::Duration::from_secs(3) {
            if let Ok(client) = self.ws_client.lock() {
                let _ = futures::executor::block_on(client.refresh_balances());
            }
            self.last_refresh = std::time::Instant::now();
        }

        let status = if let Ok(client) = self.ws_client.lock() {
            client.get_status()
        } else {
            "Error: Cannot lock client".to_string()
        };
        let is_loading = status == "Disconnected" || status.starts_with("Error");

        // Update filtered symbols when connected
        if !is_loading && self.filtered_symbols.is_empty() {
            self.update_filtered_symbols();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Binance Trading Interface");
            
            ui.horizontal(|ui| {
                ui.label("WebSocket Status:");
                ui.add(egui::Label::new(&status));
                if is_loading {
                    ui.spinner();
                }
            });

            // Rest of the UI is disabled while loading
            if is_loading {
                ui.disable();
            }

            let exchange_info = if let Ok(client) = self.ws_client.lock() {
                client.get_exchange_info()
            } else {
                HashMap::new()
            };

            self.render_modals(ui);

            ui.horizontal(|ui| {
                ui.label("API Endpoint:");
                ui.add(egui::Label::new(&self.endpoint));
            });

            ui.horizontal(|ui| {
                ui.label("WebSocket Endpoint:");
                ui.add(egui::Label::new(&self.ws_endpoint));
            });

            ui.separator();

            // Asset balances section
            ui.heading("Asset Balances");
            egui::ScrollArea::vertical()
                .id_salt("balances_scroll")
                .show(ui, |ui| {
                    tokio::task::block_in_place(|| {
                        if let Ok(client) = self.ws_client.lock() {
                            if let (Ok(balances), Ok(prices)) = (
                                futures::executor::block_on(client.get_balances()),
                                futures::executor::block_on(client.get_prices())
                            ) {
                                let mut sorted_assets: Vec<_> = balances.iter().collect();
                                sorted_assets.sort_by(|a, b| a.0.cmp(b.0));
                                
                                // Initialize portfolio totals
                                let mut portfolio_usdt = 0.0;
                                let mut portfolio_btc = 0.0;
                                let mut portfolio_usdc = 0.0;

                                for (asset, balance) in sorted_assets {
                                    if balance.free > 0.0 || balance.locked > 0.0 {
                                        ui.horizontal(|ui| {
                                            ui.add(egui::Label::new(format!("{}: ", asset)));
                                            ui.button("Trade").on_hover_text(format!("Trade {}", asset)).clicked().then(||{
                                                if let Some(price_data) = prices.get(asset) {
                                                    self.selected_symbols.insert(asset.clone(), price_data.clone());
                                                }
                                                else {
                                                    self.selected_symbols.remove(asset);
                                                }
                                            });
                                            ui.add(egui::Label::new(format!("Free: {:.8}", balance.free)));
                                            if balance.locked > 0.0 {
                                                ui.add(egui::Label::new(format!("Locked: {:.8}", balance.locked)));
                                            }
                                            if balance.total_in_usdt > 0.0 {
                                                ui.add(egui::Label::new(format!("≈ ${:.2}", balance.total_in_usdt)));
                                            }
                                            if balance.total_in_btc > 0.0 {
                                                ui.add(egui::Label::new(format!("≈ ₿{:.8}", balance.total_in_btc)));
                                            }
                                            if balance.total_in_usdc > 0.0 {
                                                ui.add(egui::Label::new(format!("≈ USDC {:.2}", balance.total_in_usdc)));
                                            }
                                        });
                                        
                                        // Add to portfolio totals
                                        portfolio_usdt += balance.total_in_usdt;
                                        portfolio_btc += balance.total_in_btc;
                                        portfolio_usdc += balance.total_in_usdc;
                                    }
                                }

                                ui.separator();
                                ui.heading("Portfolio Total");
                                ui.horizontal(|ui| {
                                    ui.label(format!("USDT: ${:.2}", portfolio_usdt));
                                    ui.label(format!("BTC: ₿{:.8}", portfolio_btc));
                                    ui.label(format!("USDC: {:.2}", portfolio_usdc));
                                });
                            }
                        }
                    });
                });

            // Market Data and Trades section
            ui.horizontal(|ui| {
                // Left side - Market Data
                ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(false), |ui| {
                    let market_panel = ui.available_width() / 2.0;
                    ui.allocate_ui_with_layout(egui::vec2(market_panel, ui.available_height()), 
                        egui::Layout::top_down_justified(egui::Align::LEFT), 
                        |ui| {
                            // Market data content...
                            self.render_market_data(ui);
                        }
                    );

                    ui.separator();

                    // Right side - Trades
                    ui.allocate_ui_with_layout(egui::vec2(market_panel, ui.available_height()),
                        egui::Layout::top_down_justified(egui::Align::LEFT),
                        |ui| self.render_trades_view(ui)
                    );
                });
            });
        });
        
        ctx.request_repaint();
    }
}

