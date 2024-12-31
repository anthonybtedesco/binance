use eframe::egui;
use log::{debug, info, error};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::sync::Mutex;
use crate::ws_client::{PriceData, SymbolInfo, WsClient};
use crate::trade::{OrderSide, OrderState, OrderTracker, OrderType, TradeOrder};
use crate::trailing::{TrailingOrder, TrailingOrderMap};
use tokio::sync::Mutex as TokioMutex;

pub struct BinanceApp {
    api_key: String,
    secret_key: String,
    endpoint: String,
    ws_endpoint: String,
    ws_client: Arc<Mutex<WsClient>>,
    market_filter: String,
    filtered_symbols: Vec<String>,
    last_refresh: std::time::Instant,
    selected_symbols: Vec<String>,
    trade_order: TradeOrder,
    order_tracker: OrderTracker,
    trailing_orders: Arc<TokioMutex<HashMap<String, TrailingOrder>>>,
}

impl BinanceApp {
    pub fn new(ws_client: Arc<Mutex<WsClient>>) -> Self {
        let order_tracker = TradeOrder::new_tracker();
        
        // Get trailing orders from WsClient instead of creating new
        let trailing_orders = if let Ok(client) = ws_client.lock() {
            client.get_trailing_orders()
        } else {
            Arc::new(TokioMutex::new(HashMap::new()))
        };

        let mut app = Self {
            api_key: env::var("API_KEY").unwrap_or_default(),
            secret_key: env::var("SECRET_KEY").unwrap_or_default(),
            endpoint: env::var("BINANCE_API_ENDPOINT").unwrap_or_default(),
            ws_endpoint: env::var("BINANCE_WS_ENDPOINT").unwrap_or_default(),
            ws_client: ws_client.clone(),
            market_filter: String::new(),
            filtered_symbols: Vec::new(),
            last_refresh: std::time::Instant::now(),
            selected_symbols: Vec::new(),
            trade_order: TradeOrder::default(),
            order_tracker: order_tracker.clone(),
            trailing_orders,
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
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .min_scrolled_height(500.0)
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
                                        self.selected_symbols.push(symbol.clone());
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
            .min_scrolled_height(500.0)
            .show(ui, |ui| {
                tokio::task::block_in_place(|| {
                    if let Ok(orders) = self.order_tracker.lock() {
                        let mut sorted_orders: Vec<_> = orders.values().collect();
                        sorted_orders.sort_by(|a, b| b.last_update.cmp(&a.last_update));

                        for order in sorted_orders {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}: ", order.symbol));
                                let side_color = match order.side {
                                    OrderSide::BUY => egui::Color32::GREEN,
                                    OrderSide::SELL => egui::Color32::RED,
                                };
                                ui.colored_label(side_color, format!("{:?}", order.side));
                                ui.label(format!("Price: {:.8}", order.price));
                                ui.label(format!("Qty: {:.8}", order.quantity));
                                ui.label(format!("Filled: {:.8}", order.filled));
                                if let Some(exec_price) = order.last_executed_price {
                                    ui.label(format!("@ {:.8}", exec_price));
                                }
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

    fn render_portfolio_total(&mut self, ui: &mut egui::Ui) {
        ui.heading("Asset Balances");
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Ok(client) = self.ws_client.lock() {
                if let (Ok(balances), Ok(prices)) = (
                    futures::executor::block_on(client.get_balances()),
                    futures::executor::block_on(client.get_prices())
                ) {
                    // Collect balances once at the start
                    let sorted_balances: Vec<_> = balances.into_iter()
                        .filter(|(_, balance)| balance.free > 0.0 || balance.locked > 0.0)
                        .collect();

                    // Calculate totals first
                    let total_usdt = sorted_balances.iter()
                        .map(|(_, balance)| balance.total_in_usdt)
                        .sum::<f64>();
                    let total_btc = sorted_balances.iter()
                        .map(|(_, balance)| balance.total_in_btc)
                        .sum::<f64>();

                    // Display individual balances
                    for (asset, balance) in &sorted_balances {
                        if asset != "USDT" {  // Don't show convert button for USDT
                            ui.horizontal(|ui| {
                                ui.label(format!("{}", asset));
                                ui.button("Trade").on_hover_text(format!("Trade {}", asset)).clicked().then(|| {
                                    self.selected_symbols.push(format!("{}USDT", asset));
                                });
                                if balance.free > 0.0 {
                                    ui.label(format!("Free: {:.8}", balance.free));
                                    
                                    // Convert to USDT button
                                    if ui.button("Convert to USDT")
                                        .on_hover_text("Sell all available balance for USDT at market price")
                                        .clicked() 
                                    {
                                        let symbol = format!("{}USDT", asset);
                                        let mut order = TradeOrder {
                                            symbol,
                                            side: OrderSide::SELL,
                                            order_type: OrderType::MARKET,
                                            quantity: balance.free,
                                            price: None,
                                        };
                                        
                                        if let Err(e) = futures::executor::block_on(order.submit(self.order_tracker.clone())) {
                                            error!("Failed to convert {} to USDT: {}", asset, e);
                                        } else {
                                            info!("Converting {} {} to USDT", balance.free, asset);
                                        }
                                    }

                                    // Convert to BTC button
                                    if asset != "BTC" && ui.button("Convert to BTC")
                                        .on_hover_text("Sell all available balance for BTC at market price")
                                        .clicked() 
                                    {
                                        let symbol = format!("{}BTC", asset);
                                        let mut order = TradeOrder {
                                            symbol,
                                            side: OrderSide::SELL,
                                            order_type: OrderType::MARKET,
                                            quantity: balance.free,
                                            price: None,
                                        };
                                        
                                        if let Err(e) = futures::executor::block_on(order.submit(self.order_tracker.clone())) {
                                            error!("Failed to convert {} to BTC: {}", asset, e);
                                        } else {
                                            info!("Converting {} {} to BTC", balance.free, asset);
                                        }
                                    }
                                }
                                if balance.locked > 0.0 {
                                    ui.label(format!("Locked: {:.8}", balance.locked));
                                }
                                if balance.total_in_usdt > 0.0 {
                                    ui.label(format!("≈ ${:.2}", balance.total_in_usdt));
                                }
                                if balance.total_in_btc > 0.0 {
                                    ui.label(format!("≈ ₿{:.8}", balance.total_in_btc));
                                }
                            });
                        }
                    }

                    // Portfolio summary section
                    ui.separator();
                    ui.heading("Portfolio Summary");
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(format!("Total USDT: ${:.2}", total_usdt));
                            if ui.button("Convert All to USDT")
                                .on_hover_text("Convert all non-USDT assets to USDT")
                                .clicked() 
                            {
                                for (asset, balance) in &sorted_balances {
                                    if asset != "USDT" && balance.free > 0.0 {
                                        let symbol = format!("{}USDT", asset);
                                        let mut order = TradeOrder {
                                            symbol,
                                            side: OrderSide::SELL,
                                            order_type: OrderType::MARKET,
                                            quantity: balance.free,
                                            price: None,
                                        };
                                        if let Err(e) = futures::executor::block_on(order.submit(self.order_tracker.clone())) {
                                            error!("Failed to convert {} to USDT: {}", asset, e);
                                        }
                                    }
                                }
                            }
                        });

                        ui.vertical(|ui| {
                            ui.label(format!("Total BTC: ₿{:.8}", total_btc));
                            if ui.button("Convert All to BTC")
                                .on_hover_text("Convert all non-BTC assets to BTC")
                                .clicked() 
                            {
                                for (asset, balance) in &sorted_balances {
                                    if asset != "BTC" && balance.free > 0.0 {
                                        let symbol = format!("{}BTC", asset);
                                        let mut order = TradeOrder {
                                            symbol,
                                            side: OrderSide::SELL,
                                            order_type: OrderType::MARKET,
                                            quantity: balance.free,
                                            price: None,
                                        };
                                        if let Err(e) = futures::executor::block_on(order.submit(self.order_tracker.clone())) {
                                            error!("Failed to convert {} to BTC: {}", asset, e);
                                        }
                                    }
                                }
                            }
                        });
                    });

                    ui.separator();
                    let total_value = if let Some(btc_price) = prices.get("BTCUSDT").and_then(|p| {
                        // Check if price update is recent (within last minute)
                        if p.last_update.timestamp() > chrono::Utc::now().timestamp() - 60 {
                            Some(p.price)
                        } else {
                            None
                        }
                    }) {
                        total_usdt + (total_btc * btc_price)
                    } else {
                        total_usdt // Fall back to just USDT total if BTC price is unavailable/stale
                    };

                    ui.heading(format!("Portfolio Total: ${:.2}", total_value));
                    ui.horizontal(|ui| {
                        ui.label(format!("Total in USDT: ${:.2}", total_value));
                        if let Some(btc_price) = prices.get("BTCUSDT").map(|p| p.price) {
                            let total_value_btc = total_value / btc_price;
                            ui.label(format!("Total in BTC: ₿{:.8}", total_value_btc));
                        } else {
                            ui.label(format!("Total in BTC: ₿{:.8}", total_btc));
                        }
                    });
                }
            }
        });
    }

    fn render_trade_modals(&mut self, ctx: &egui::Context) {
        // Make windows bigger
        let modal_style = egui::Style {
            spacing: egui::style::Spacing {
                item_spacing: egui::vec2(10.0, 10.0),
                button_padding: egui::vec2(10.0, 5.0),
                ..Default::default()
            },
            text_styles: {
                use egui::TextStyle::*;
                let mut ts = std::collections::BTreeMap::new();
                ts.insert(Body, egui::FontId::new(16.0, egui::FontFamily::Proportional));
                ts.insert(Button, egui::FontId::new(16.0, egui::FontFamily::Proportional));
                ts.insert(Heading, egui::FontId::new(22.0, egui::FontFamily::Proportional));
                ts.insert(Small, egui::FontId::new(14.0, egui::FontFamily::Proportional));
                ts
            },
            ..Default::default()
        };

        if let Ok(client) = self.ws_client.lock() {
            if let (Ok(prices), Ok(exchange_info)) = (
                futures::executor::block_on(client.get_prices()),
                futures::executor::block_on(client.get_exchange_info())
            ) {
                let selected = self.selected_symbols.clone();
                for symbol in selected {
                    let mut is_open = true;
                    if let (Some(price_data), Some(symbol_info)) = (prices.get(&symbol), exchange_info.symbols.get(&symbol)) {
                        egui::Window::new(format!("Trade {}", symbol))
                            .open(&mut is_open)
                            .min_width(400.0)
                            .show(ctx, |ui| {
                                ui.heading("Market Information");
                                ui.horizontal(|ui| {
                                    ui.label(format!("Current Price: {:.8}", price_data.price));
                                    ui.label(format!("24h Range: {:.8} - {:.8}", price_data.low_24h, price_data.high_24h));
                                });

                                ui.separator();
                                ui.heading("Place Order");
                                
                                ui.horizontal(|ui| {
                                    if ui.selectable_value(&mut self.trade_order.side, OrderSide::BUY, "Buy").clicked() ||
                                       ui.selectable_value(&mut self.trade_order.side, OrderSide::SELL, "Sell").clicked() {
                                        // Update quantity based on price when side changes
                                        if let Some(price) = self.trade_order.price {
                                            self.trade_order.quantity = 100.0 / price; // Example: $100 worth
                                        }
                                    }
                                });

                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut self.trade_order.order_type, OrderType::LIMIT, "Limit");
                                    ui.selectable_value(&mut self.trade_order.order_type, OrderType::MARKET, "Market");
                                    
                                    // Add trailing order buttons
                                    ui.horizontal(|ui| {
                                        let mut delta = 1.0;
                                        ui.label("Trail %:");
                                        if ui.add(egui::Slider::new(&mut delta, 0.1..=10.0)
                                            .text("Trailing %")
                                            .clamping(egui::SliderClamping::Always)
                                            .smart_aim(true)
                                        ).changed() {
                                            // Keep the delta value updated
                                        }

                                        if ui.button("Trailing Stop").clicked() {
                                            info!("Creating trailing stop with {}%", delta);
                                            tokio::task::block_in_place(|| {
                                                if let Ok(mut trails) = self.trailing_orders.try_lock() {
                                                    let trail = TrailingOrder::new(
                                                        symbol.clone(),
                                                        OrderSide::SELL,
                                                        self.trade_order.quantity,
                                                        delta,
                                                        price_data.price,
                                                    );
                                                    let id = format!("{}_{}", symbol, chrono::Utc::now().timestamp());
                                                    trails.insert(id, trail.clone());
                                                    if let Err(e) = TrailingOrder::save_trails(&trails) {
                                                        error!("Failed to save new trailing order: {}", e);
                                                    }
                                                } else {
                                                    error!("Failed to acquire trailing orders lock");
                                                }
                                            });
                                        }

                                        if ui.button("Trailing Buy").clicked() {
                                            info!("Creating trailing buy with {}%", delta);
                                            tokio::task::block_in_place(|| {
                                                if let Ok(mut trails) = self.trailing_orders.try_lock() {
                                                    let trail = TrailingOrder::new(
                                                        symbol.clone(),
                                                        OrderSide::BUY,
                                                        self.trade_order.quantity,
                                                        delta,
                                                        price_data.price,
                                                    );
                                                    let id = format!("{}_{}", symbol, chrono::Utc::now().timestamp());
                                                    trails.insert(id, trail.clone());
                                                    if let Err(e) = TrailingOrder::save_trails(&trails) {
                                                        error!("Failed to save new trailing order: {}", e);
                                                    }
                                                } else {
                                                    error!("Failed to acquire trailing orders lock");
                                                }
                                            });
                                        }
                                    });
                                });

                                if let OrderType::LIMIT = self.trade_order.order_type {
                                    ui.horizontal(|ui| {
                                        ui.label("Price:");
                                        let mut price = self.trade_order.price.unwrap_or(price_data.price);
                                        if ui.add(egui::DragValue::new(&mut price)
                                            .speed(symbol_info.tick_size)
                                            .prefix("$")
                                            .range(symbol_info.min_price..=symbol_info.max_price)
                                        ).changed() {
                                            self.trade_order.price = Some(price);
                                            // Update quantity when price changes
                                            self.trade_order.quantity = 100.0 / price; // Example: $100 worth
                                        }
                                    });
                                }

                                ui.horizontal(|ui| {
                                    ui.label("Quantity:");
                                    let mut quantity = self.trade_order.quantity;
                                    if ui.add(egui::DragValue::new(&mut quantity)
                                        .speed(symbol_info.step_size)
                                        .range(symbol_info.min_qty..=symbol_info.max_qty)
                                    ).changed() {
                                        self.trade_order.quantity = quantity;
                                    }
                                    
                                    // Show value in USD
                                    if let Some(price) = self.trade_order.price.or(Some(price_data.price)) {
                                        ui.label(format!("≈ ${:.2}", quantity * price));
                                    }
                                });

                                // Add percentage buttons
                                ui.horizontal(|ui| {
                                    for percentage in [25, 50, 75, 100] {
                                        if ui.button(format!("{}%", percentage)).clicked() {
                                            if let Some(price) = self.trade_order.price.or(Some(price_data.price)) {
                                                let value = 100.0 * (percentage as f64 / 100.0);
                                                self.trade_order.quantity = value / price;
                                            }
                                        }
                                    }
                                });

                                if ui.button("Place Order").clicked() {
                                    info!("Preparing to submit order for {}", symbol);
                                    let mut order = TradeOrder {
                                        symbol: symbol.clone(),
                                        side: self.trade_order.side.clone(),
                                        order_type: self.trade_order.order_type.clone(),
                                        quantity: self.trade_order.quantity,
                                        price: self.trade_order.price,
                                    };
                                    
                                    // Ensure we have a price for limit orders
                                    if let OrderType::LIMIT = order.order_type {
                                        if order.price.is_none() {
                                            order.price = Some(price_data.price);
                                        }
                                    }

                                    info!("Order details: {:?}", order);
                                    
                                    // Submit the order
                                    if let Err(e) = futures::executor::block_on(order.submit(self.order_tracker.clone())) {
                                        error!("Failed to submit order: {}", e);
                                    } else {
                                        info!("Order submitted successfully");
                                        self.selected_symbols.retain(|s| s != &symbol);
                                    }
                                }

                                ui.separator();
                                ui.heading("Active Trailing Orders");
                                
                                if let Ok(trails) = self.trailing_orders.try_lock() {
                                    let symbol_trails: Vec<_> = trails.iter()
                                        .filter(|(_, trail)| trail.symbol == symbol)
                                        .collect();

                                    if symbol_trails.is_empty() {
                                        ui.label("No active trailing orders");
                                    } else {
                                        egui::Grid::new("trailing_orders_grid")
                                            .striped(true)
                                            .spacing([10.0, 4.0])
                                            .show(ui, |ui| {
                                                // Headers
                                                ui.label("Side");
                                                ui.label("Quantity");
                                                ui.label("Delta %");
                                                ui.label("Initial Price");
                                                ui.label("Current");
                                                ui.label("Trigger");
                                                ui.label("Value (USDT)");
                                                ui.label("Age");
                                                ui.end_row();

                                                for (id, trail) in symbol_trails {
                                                    let side_color = match trail.side {
                                                        OrderSide::BUY => egui::Color32::GREEN,
                                                        OrderSide::SELL => egui::Color32::RED,
                                                    };
                                                    ui.colored_label(side_color, format!("{:?}", trail.side));
                                                    ui.label(format!("{:.8}", trail.quantity));
                                                    ui.label(format!("{:.2}%", trail.delta_percentage));
                                                    ui.label(format!("{:.8}", trail.initial_price));
                                                    ui.label(format!("{:.8}", trail.last_price));
                                                    ui.label(format!("{:.8}", trail.trigger_price));
                                                    ui.label(format!("${:.2}", trail.usdt_value));
                                                    ui.label(format!("{}", humantime::format_duration(
                                                        std::time::SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .unwrap()
                                                            .saturating_sub(std::time::Duration::from_secs(
                                                                trail.created_at.timestamp() as u64
                                                            ))
                                                    )));
                                                    if ui.button("Cancel").clicked() {
                                                        info!("Cancelling trailing order: {}", id);
                                                        if let Ok(mut trails) = self.trailing_orders.try_lock() {
                                                            trails.remove(id);
                                                            if let Err(e) = TrailingOrder::save_trails(&trails) {
                                                                error!("Failed to save trails after removal: {}", e);
                                                            }
                                                        }
                                                    }
                                                    ui.end_row();
                                                }
                                                
                                            });
                                    }
                                }
                            });

                        if !is_open {
                            self.selected_symbols.retain(|s| s != &symbol);
                        }
                    }
                }
            }
        }
    }

    fn render_trailing_orders(&mut self, ui: &mut egui::Ui) {
        ui.heading("Trailing Orders");
        
        egui::ScrollArea::vertical()
            .id_salt("trailing_scroll")
            .min_scrolled_height(500.0)
            .show(ui, |ui| {
                tokio::task::block_in_place(|| {
                    let trails = futures::executor::block_on(self.trailing_orders.lock());
                    if trails.is_empty() {
                        ui.label("No active trailing orders");
                    } else {
                        for (id, trail) in trails.iter() {
                            ui.horizontal(|ui| {
                                let side_color = match trail.side {
                                    OrderSide::BUY => egui::Color32::GREEN,
                                    OrderSide::SELL => egui::Color32::RED,
                                };
                                ui.colored_label(side_color, format!("{:?}", trail.side));
                                ui.label(format!("{}: ", trail.symbol));
                                ui.label(format!("Qty: {:.8}", trail.quantity));
                                ui.label(format!("Delta: {:.2}%", trail.delta_percentage));
                                ui.label(format!("Current: {:.8}", trail.last_price));
                                ui.label(format!("Trigger: {:.8}", trail.trigger_price));
                                ui.label(format!("Value: ${:.2}", trail.usdt_value));
                                
                                if ui.button("Cancel").clicked() {
                                    let mut trails = futures::executor::block_on(self.trailing_orders.lock());
                                    trails.remove(id);
                                    if let Err(e) = TrailingOrder::save_trails(&trails) {
                                        error!("Failed to save trails after removal: {}", e);
                                    }
                                }
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
                futures::executor::block_on(client.get_exchange_info()).unwrap_or_default().symbols
            } else {
                HashMap::new()
            };

            ui.horizontal(|ui| {
                ui.label("API Endpoint:");
                ui.add(egui::Label::new(&self.endpoint));
            });

            ui.horizontal(|ui| {
                ui.label("WebSocket Endpoint:");
                ui.add(egui::Label::new(&self.ws_endpoint));
            });

            ui.label(format!("Selected Symbols: {}", self.selected_symbols.join(", ")));

            ui.separator();

            self.render_portfolio_total(ui);

            ui.separator();

            self.render_trailing_orders(ui);

            ui.separator();

            // Market Data and Trades section
            ui.horizontal(|ui| {
                // Left side - Market Data
                ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(false), |ui| {
                    let market_panel = ui.available_width() / 2.0;
                    let available_height = ui.available_height();

                    // Market data panel
                    ui.vertical(|ui| {
                        ui.set_min_height(available_height);
                        ui.allocate_ui_with_layout(
                            egui::vec2(market_panel, available_height), 
                            egui::Layout::top_down_justified(egui::Align::LEFT), 
                            |ui| {
                                self.render_market_data(ui);
                            }
                        );
                    });

                    ui.separator();

                    // Right side - Trades
                    ui.vertical(|ui| {
                        ui.set_min_height(available_height);
                        ui.allocate_ui_with_layout(
                            egui::vec2(market_panel, available_height),
                            egui::Layout::top_down_justified(egui::Align::LEFT),
                            |ui| {
                                self.render_trades_view(ui)
                            }
                        );
                    });
                });
            });
        });

        self.render_trade_modals(ctx);
        
        ctx.request_repaint();
    }
}

