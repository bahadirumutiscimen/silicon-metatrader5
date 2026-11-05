// Python API reference shown on the Education page. Every text is {tr, en}.
// The functions are those of the official MetaTrader5 package; siliconmetatrader5
// exposes the same names through the bridge, plus a few extras marked `extra`.

export const QUICKSTART = `from siliconmetatrader5 import MetaTrader5, server_now
from datetime import timedelta

mt5 = MetaTrader5(host="127.0.0.1", port=8001, keepalive=True)
if not mt5.initialize():
    raise SystemExit(f"initialize() failed: {mt5.last_error()}")

print(mt5.account_info().server, mt5.terminal_info().build)

# last 200 CLOSED M15 bars (position 1 skips the bar that is still forming)
rates = mt5.copy_rates_from_pos("EURUSD", mt5.TIMEFRAME_M15, 1, 200)
print(rates[-1])          # (time, open, high, low, close, tick_volume, spread, real_volume)

# a time range that ends "now" on the broker's clock
now = server_now(mt5)
bars = mt5.copy_rates_range("EURUSD", mt5.TIMEFRAME_M5, now - timedelta(hours=6), now)

mt5.close()               # disconnect this script only`;

// Lessons learned in real projects (closed bars, server time, one engine for backtest and live …)
export const RULES = [
  { tr: '<b>Kapanmış barla karar ver.</b> <code>copy_rates_from_pos(…, 0, n)</code> içindeki son satır hâlâ oluşan bardır; değeri her tick\'te değişir. Sinyali onunla verirsen geçmişte görünen sinyal sonradan kaybolur (repaint). Başlangıç pozisyonunu <code>1</code> yap ya da son satırı at.',
    en: '<b>Decide on closed bars.</b> The last row of <code>copy_rates_from_pos(…, 0, n)</code> is the bar that is still forming; it changes with every tick. Signals based on it disappear later (repainting). Start at position <code>1</code> or drop the last row.' },
  { tr: '<b>Saat broker sunucusunun saatidir.</b> Bar zamanları sunucu saatindedir (ör. UTC+2), bilgisayarınınkinde değil. "Şimdiye kadar" aralıkları için <code>datetime.now()</code> yerine <code>server_now(mt5)</code> kullan.',
    en: '<b>Time is the broker server\'s clock.</b> Bar times are in server time (e.g. UTC+2), not your computer\'s. For ranges that end "now", use <code>server_now(mt5)</code>, not <code>datetime.now()</code>.' },
  { tr: '<b>Backtest ve canlı aynı motoru kullansın.</b> Stratejiyi <code>on_bar(bar) → sinyal</code> biçiminde yan etkisiz yaz. Backtest geçmişi bar bar bu fonksiyondan geçirir, canlı döngü her yeni kapanan barı verir. Emir ve bildirim motorun dışında kalır. Böylece canlı sinyaller backtest\'le birebir aynı olur.',
    en: '<b>Use one engine for backtest and live.</b> Write the strategy as a side-effect free <code>on_bar(bar) → signal</code>. The backtest replays history bar by bar through it; the live loop feeds every newly closed bar. Orders and notifications stay outside. Live signals then match the backtest exactly.' },
  { tr: '<b>Sinyal bar N\'de, işlem bar N+1\'in açılışında.</b> Backtest\'te kapanış fiyatından işlem açmak geleceği görmektir; gerçekte o fiyat bar kapandıktan sonra artık yoktur.',
    en: '<b>Signal on bar N, trade at bar N+1\'s open.</b> Filling at the signal bar\'s close in a backtest looks into the future; in reality that price is gone once the bar has closed.' },
  { tr: '<b>Tek bağlantı, keepalive.</b> <code>MetaTrader5(…, keepalive=True)</code> bağlantıyı canlı tutar ve koparsa yeniden kurar. İşin bitince <code>close()</code> çağır; <code>shutdown()</code> köprüyü kullanan tüm programları koparır.',
    en: '<b>One connection, keepalive.</b> <code>MetaTrader5(…, keepalive=True)</code> keeps the connection alive and reconnects. Call <code>close()</code> when done; <code>shutdown()</code> disconnects every program that uses the bridge.' },
  { tr: '<b>Her çağrıyı kontrol et.</b> Başarısız çağrılar istisna değil <code>None</code> döndürür; nedenini <code>last_error()</code> söyler. Canlı döngüde hataları yakala ve kısa bir beklemeden sonra devam et.',
    en: '<b>Check every call.</b> Failed calls return <code>None</code> instead of raising; <code>last_error()</code> tells why. In a live loop, catch errors and continue after a short pause.' },
  { tr: '<b>MaxBars sınırı.</b> <code>copy_rates_from_pos</code> isteği MT5\'in MaxBars ayarını (varsayılan 100 000) aşarsa hiçbir şey dönmez. Daha az bar iste ya da Runtime Ayarları\'ndan artır.',
    en: '<b>MaxBars limit.</b> If a <code>copy_rates_from_pos</code> request goes past MT5\'s MaxBars (100,000 by default) it returns nothing. Ask for fewer bars or raise it in Runtime Settings.' },
  { tr: '<b>Emirleri önce demo\'da dene.</b> <code>order_check()</code> ile isteği gönderilmeden doğrula, sonucu <code>retcode</code> ile kontrol et. Gerçek hesapta küçük hacimle başla.',
    en: '<b>Try orders on demo first.</b> Validate a request with <code>order_check()</code> before sending it, and check the result\'s <code>retcode</code>. Start small on a real account.' },
];

const g = (id, tr, en, items) => ({ id, title: { tr, en }, items });
const f = (name, sig, tr, en, ex, extra = false) => ({ name, sig, desc: { tr, en }, ex, extra });

// Server mode: connecting to a VPS over SSH (wizard step + Python API page).
export const SSH_GUIDE = [
  { tr: '<b>Gerekenler:</b> Linux x86_64 (Ubuntu 22.04+) ya da Windows (10/11, Server 2019+, x64) bir VPS, IP adresi, kullanıcı adı (<code>root</code> / <code>ubuntu</code> / <code>Administrator</code>) ve şifre ya da <code>.pem</code> anahtar dosyası. Hepsi sağlayıcının panelinde ya da hoş geldin e-postasında yazar.',
    en: '<b>You need:</b> a Linux x86_64 (Ubuntu 22.04+) or Windows (10/11, Server 2019+, x64) VPS, its IP address, a user name (<code>root</code> / <code>ubuntu</code> / <code>Administrator</code>) and a password or a <code>.pem</code> key file. Your provider\'s panel or welcome email has them.' },
  { tr: '<b>Önce Terminal\'de dene:</b> <code>ssh kullanici@IP</code> (port farklıysa <code>-p 2222</code>), soruya <code>yes</code> de, şifreni gir. Giriş oluyorsa uygulama da bağlanır.',
    en: '<b>Try it in Terminal first:</b> <code>ssh user@IP</code> (another port: <code>-p 2222</code>), answer <code>yes</code>, type the password. If that logs in, the app can connect too.' },
  { tr: '<b>Uygulamaya yaz:</b> Sunucu adresine <code>kullanici@IP</code> (port farklıysa <code>kullanici@IP:2222</code>). <code>.pem</code> dosyası verildiyse yolunu Anahtar dosyası alanına yaz, yoksa boş bırak.',
    en: '<b>In the app:</b> Server address <code>user@IP</code> (another port: <code>user@IP:2222</code>). Got a <code>.pem</code> file? Put its path in Key file, otherwise leave it empty.' },
  { tr: '<b>Parmak izi</b> sunucunun kimliğidir. Sağlayıcının konsolunda <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code> çıktısıyla aynıysa onayla.',
    en: '<b>The fingerprint</b> is the server\'s identity. Accept it if it matches <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code> in your provider\'s console.' },
  { tr: '<b>Şifreyle girersen</b> Terminal açılır, şifreni bir kez orada yazarsın; uygulama bir anahtar kurar, sonra şifre gerekmez. Kullanıcının <code>sudo</code> yetkisi olmalı.',
    en: '<b>With a password,</b> Terminal opens and you type it there once; the app sets up a key, after that no password is needed. The user needs <code>sudo</code>.' },
  { tr: '<b>Hatalar:</b> <i>Permission denied</i> = anahtar sunucuda değil · <i>Connection timed out</i> = IP/port yanlış ya da güvenlik duvarı 22\'yi kapatıyor · <i>Kimlik değişti</i> = sunucuyu yeniden kurmadıysan bağlanma.',
    en: '<b>Errors:</b> <i>Permission denied</i> = the key is not on the server · <i>Connection timed out</i> = wrong IP/port or a firewall blocks 22 · <i>Identity changed</i> = do not connect unless you reinstalled the server.' },
  { tr: '<b>Windows sunucu:</b> Windows\'ta SSH önce açılmalı. Sunucuda yönetici PowerShell\'de: <code>Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0; Start-Service sshd; Set-Service sshd -StartupType Automatic</code>. SSH kullanıcısı yönetici olmalı (ör. <code>Administrator</code>). Şifreyle girersen anahtar <code>administrators_authorized_keys</code> dosyasına eklenir.',
    en: '<b>Windows server:</b> turn SSH on first. In an administrator PowerShell on the server: <code>Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0; Start-Service sshd; Set-Service sshd -StartupType Automatic</code>. The SSH user must be an administrator (e.g. <code>Administrator</code>). With a password, the key is added to <code>administrators_authorized_keys</code>.' },
];

export const GROUPS = [
  g('connect', 'Bağlantı', 'Connection', [
    f('MetaTrader5', 'MetaTrader5(host="127.0.0.1", port=8001, keepalive=False, timeout=None)',
      'Köprüye bağlanan istemci. keepalive=True arka planda bağlantıyı kontrol eder ve koparsa yeniden kurar; uzun çalışan programlar için önerilir.',
      'Client connected to the bridge. keepalive=True checks the connection in the background and reconnects; recommended for long-running programs.',
      'mt5 = MetaTrader5(host="127.0.0.1", port=8001, keepalive=True)', true),
    f('initialize', 'initialize(path=None, login=None, password=None, server=None, timeout=None, portable=False) → bool',
      'MT5 terminaliyle bağlantıyı açar. SiliconMetaTrader5\'te terminal zaten açık ve girişli olduğu için parametresiz çağır.',
      'Opens the connection to the MT5 terminal. In SiliconMetaTrader5 the terminal is already running and logged in, so call it without arguments.',
      'if not mt5.initialize():\n    print(mt5.last_error())'),
    f('login', 'login(login, password=None, server=None, timeout=60000) → bool',
      'Terminalde başka bir hesaba geçer. Terminalin girişini değiştirir; köprüyü kullanan tüm programları etkiler.',
      'Switches the terminal to another account. It changes the terminal\'s login for every program using the bridge.',
      'mt5.login(12345678, password="…", server="Broker-Demo")'),
    f('close', 'close(remote_shutdown=False)', 'Yalnız bu programın bağlantısını kapatır. Programın sonunda çağır.',
      'Closes only this program\'s connection. Call it at the end of your program.', 'mt5.close()', true),
    f('shutdown', 'shutdown() → None', 'Terminalle bağlantıyı tüm istemciler için kapatır. Genelde close() istediğin şeydir.',
      'Closes the terminal connection for every client. close() is usually what you want.', 'mt5.shutdown()'),
    f('version', 'version() → (int, int, str)', 'Terminal sürümü, build ve tarih.', 'Terminal version, build and date.', 'print(mt5.version())'),
    f('last_error', 'last_error() → (int, str)', 'Son başarısız çağrının hata kodu ve açıklaması.', 'Error code and description of the last failed call.', 'code, msg = mt5.last_error()'),
    f('ping', 'ping() → bool', 'Köprü bağlantısı canlı mı? Hızlıdır, MT5\'e gitmez.', 'Is the bridge connection alive? Fast; does not reach MT5.', 'if not mt5.ping():\n    print("bridge down")', true),
  ]),
  g('account', 'Hesap ve terminal', 'Account and terminal', [
    f('account_info', 'account_info() → AccountInfo', 'Hesap: login, server, balance, equity, margin, margin_free, leverage, currency, trade_mode (0 demo, 2 gerçek)…',
      'Account: login, server, balance, equity, margin, margin_free, leverage, currency, trade_mode (0 demo, 2 real)…',
      'a = mt5.account_info()\nprint(a.balance, a.equity, a.currency)'),
    f('terminal_info', 'terminal_info() → TerminalInfo', 'Terminal: connected (sunucuya bağlı mı), trade_allowed, build, maxbars, dlls_allowed…',
      'Terminal: connected (to the server), trade_allowed, build, maxbars, dlls_allowed…', 'print(mt5.terminal_info().connected)'),
  ]),
  g('symbols', 'Semboller', 'Symbols', [
    f('symbols_total', 'symbols_total() → int', 'Broker\'daki sembol sayısı.', 'Number of symbols at the broker.', 'print(mt5.symbols_total())'),
    f('symbols_get', 'symbols_get(group="*") → tuple[SymbolInfo]', 'Sembolleri getirir; group ile filtrele ("*USD*", "!*JPY*").',
      'Returns symbols; filter with group ("*USD*", "!*JPY*").', 'names = [s.name for s in mt5.symbols_get("*USD*")]'),
    f('symbol_info', 'symbol_info(symbol) → SymbolInfo', 'Sembol özellikleri: digits, point, trade_contract_size, volume_min/step, spread, filling_mode…',
      'Symbol properties: digits, point, trade_contract_size, volume_min/step, spread, filling_mode…', 'i = mt5.symbol_info("EURUSD")\nprint(i.digits, i.point, i.volume_min)'),
    f('symbol_info_tick', 'symbol_info_tick(symbol) → Tick', 'Son fiyat: time, bid, ask, last, volume (time sunucu saatinde).',
      'Latest price: time, bid, ask, last, volume (time on the server clock).', 't = mt5.symbol_info_tick("EURUSD")\nprint(t.bid, t.ask)'),
    f('symbol_select', 'symbol_select(symbol, enable=True) → bool', 'Sembolü Market Watch\'a ekler. Veri istemeden önce çağır.',
      'Adds the symbol to Market Watch. Call it before asking for data.', 'mt5.symbol_select("XAUUSD", True)'),
  ]),
  g('book', 'Piyasa derinliği', 'Market depth', [
    f('market_book_add', 'market_book_add(symbol) → bool', 'Derinlik (DOM) aboneliği başlatır; broker desteklemeli.', 'Subscribes to market depth (DOM); the broker must support it.', 'mt5.market_book_add("EURUSD")'),
    f('market_book_get', 'market_book_get(symbol) → tuple[BookInfo]', 'Derinlik satırları: type, price, volume.', 'Depth rows: type, price, volume.', 'for r in mt5.market_book_get("EURUSD") or []:\n    print(r.type, r.price, r.volume)'),
    f('market_book_release', 'market_book_release(symbol) → bool', 'Aboneliği bitirir.', 'Ends the subscription.', 'mt5.market_book_release("EURUSD")'),
  ]),
  g('rates', 'Fiyat geçmişi', 'Price history', [
    f('copy_rates_from_pos', 'copy_rates_from_pos(symbol, timeframe, start_pos, count) → ndarray',
      'Sondan geriye bar: start_pos 0 oluşan bar, 1 son kapanmış bar. Canlı strateji için en pratik yol.',
      'Bars counted back from the newest: start_pos 0 is the forming bar, 1 the last closed one. The most practical call for live strategies.',
      'r = mt5.copy_rates_from_pos("EURUSD", mt5.TIMEFRAME_H1, 1, 500)\nlast_closed = r[-1]'),
    f('copy_rates_from', 'copy_rates_from(symbol, timeframe, date_from, count) → ndarray',
      'date_from anından geriye count bar. Backtest\'te belirli bir tarihe kadar veri için.', 'count bars back from date_from. For backtests up to a given date.',
      'from datetime import datetime\nr = mt5.copy_rates_from("EURUSD", mt5.TIMEFRAME_M15, datetime(2026, 10, 1), 2000)'),
    f('copy_rates_range', 'copy_rates_range(symbol, timeframe, date_from, date_to) → ndarray',
      'İki tarih arasındaki barlar. Uzun aralıkları parça parça iste; tek istek MaxBars\'a takılabilir.',
      'Bars between two dates. Ask for long ranges in chunks; one request can hit MaxBars.',
      'now = server_now(mt5)\nr = mt5.copy_rates_range("EURUSD", mt5.TIMEFRAME_M5, now - timedelta(days=2), now)'),
    f('copy_ticks_from', 'copy_ticks_from(symbol, date_from, count, flags) → ndarray', 'date_from\'dan itibaren count tick. flags: COPY_TICKS_ALL, COPY_TICKS_INFO, COPY_TICKS_TRADE.',
      'count ticks from date_from on. flags: COPY_TICKS_ALL, COPY_TICKS_INFO, COPY_TICKS_TRADE.', 'ticks = mt5.copy_ticks_from("EURUSD", now - timedelta(minutes=5), 1000, mt5.COPY_TICKS_ALL)'),
    f('copy_ticks_range', 'copy_ticks_range(symbol, date_from, date_to, flags) → ndarray', 'İki tarih arasındaki tickler.', 'Ticks between two dates.',
      'ticks = mt5.copy_ticks_range("EURUSD", now - timedelta(minutes=10), now, mt5.COPY_TICKS_INFO)'),
  ]),
  g('trade', 'Emirler ve pozisyonlar', 'Orders and positions', [
    f('order_calc_margin', 'order_calc_margin(action, symbol, volume, price) → float', 'İşlemin gerektireceği teminat (hesap para biriminde).',
      'Margin the trade would need (in account currency).', 'm = mt5.order_calc_margin(mt5.ORDER_TYPE_BUY, "EURUSD", 0.1, tick.ask)'),
    f('order_calc_profit', 'order_calc_profit(action, symbol, volume, price_open, price_close) → float', 'Verilen fiyatlar arasında kâr/zarar.',
      'Profit or loss between two prices.', 'p = mt5.order_calc_profit(mt5.ORDER_TYPE_BUY, "EURUSD", 0.1, 1.0800, 1.0850)'),
    f('order_check', 'order_check(request) → OrderCheckResult', 'İsteği göndermeden doğrular (teminat, hacim, fiyat).',
      'Validates a request without sending it (margin, volume, price).', 'r = mt5.order_check(request)\nprint(r.retcode, r.comment)'),
    f('order_send', 'order_send(request) → OrderSendResult', 'Emir gönderir. Başarı: retcode == TRADE_RETCODE_DONE. Hesabına gerçek işlem açar; önce demo\'da dene.',
      'Sends an order. Success: retcode == TRADE_RETCODE_DONE. It trades your account for real; try it on demo first.',
      'request = {"action": mt5.TRADE_ACTION_DEAL, "symbol": "EURUSD", "volume": 0.01,\n           "type": mt5.ORDER_TYPE_BUY, "price": tick.ask, "deviation": 20,\n           "magic": 1001, "type_time": mt5.ORDER_TIME_GTC, "type_filling": mt5.ORDER_FILLING_IOC}\nres = mt5.order_send(request)'),
    f('orders_total', 'orders_total() → int', 'Bekleyen emir sayısı.', 'Number of pending orders.', 'print(mt5.orders_total())'),
    f('orders_get', 'orders_get(symbol=None, group=None, ticket=None) → tuple[TradeOrder]', 'Bekleyen emirler.', 'Pending orders.', 'for o in mt5.orders_get(symbol="EURUSD") or []:\n    print(o.ticket, o.type, o.price_open)'),
    f('positions_total', 'positions_total() → int', 'Açık pozisyon sayısı.', 'Number of open positions.', 'print(mt5.positions_total())'),
    f('positions_get', 'positions_get(symbol=None, group=None, ticket=None) → tuple[TradePosition]', 'Açık pozisyonlar: ticket, type, volume, price_open, sl, tp, profit, magic.',
      'Open positions: ticket, type, volume, price_open, sl, tp, profit, magic.', 'for p in mt5.positions_get(symbol="EURUSD") or []:\n    print(p.ticket, p.volume, p.profit)'),
  ]),
  g('history', 'İşlem geçmişi', 'Trade history', [
    f('history_orders_total', 'history_orders_total(date_from, date_to) → int', 'Aralıktaki geçmiş emir sayısı.', 'Number of historical orders in the range.', 'n = mt5.history_orders_total(now - timedelta(days=7), now)'),
    f('history_orders_get', 'history_orders_get(date_from, date_to, group=None) | (ticket=…) | (position=…)', 'Geçmiş emirler.', 'Historical orders.', 'orders = mt5.history_orders_get(now - timedelta(days=7), now)'),
    f('history_deals_total', 'history_deals_total(date_from, date_to) → int', 'Aralıktaki gerçekleşen işlem (deal) sayısı.', 'Number of deals in the range.', 'n = mt5.history_deals_total(now - timedelta(days=7), now)'),
    f('history_deals_get', 'history_deals_get(date_from, date_to, group=None) | (ticket=…) | (position=…)', 'Gerçekleşen işlemler: profit, commission, swap, price, time.',
      'Deals: profit, commission, swap, price, time.', 'deals = mt5.history_deals_get(now - timedelta(days=30), now)\nprint(sum(d.profit for d in deals or []))'),
  ]),
  g('time', 'Sunucu saati yardımcıları', 'Server time helpers', [
    f('server_now', 'server_now(mt5, symbol="EURUSD", offset=None) → datetime', 'Broker sunucusunun şu anki saati; aralık sorgularında "şimdi" olarak kullan.',
      'The broker server\'s current time; use it as "now" in range queries.', 'from siliconmetatrader5 import server_now\nnow = server_now(mt5)', true),
    f('server_time_offset', 'server_time_offset(mt5, symbol="EURUSD") → int | None', 'Sunucu saatinin UTC\'den farkı (saniye), son tickten hesaplanır.',
      'Offset of the server clock from UTC (seconds), measured from recent ticks.', 'from siliconmetatrader5 import server_time_offset, format_offset\nprint(format_offset(server_time_offset(mt5)))', true),
    f('bar_time', 'bar_time(epoch_seconds) → datetime', 'Bar zamanını (sunucu saati) datetime\'a çevirir.', 'Converts a bar time (server clock) to a datetime.',
      'from siliconmetatrader5 import bar_time\nprint(bar_time(int(r[-1]["time"])))', true),
  ]),
];

export const CONSTANTS = [
  ['TIMEFRAME_M1 … M30, H1, H4, D1, W1, MN1', { tr: 'Bar periyotları', en: 'Bar periods' }],
  ['ORDER_TYPE_BUY / SELL / BUY_LIMIT / SELL_LIMIT / BUY_STOP / SELL_STOP', { tr: 'Emir türleri', en: 'Order types' }],
  ['TRADE_ACTION_DEAL / PENDING / SLTP / MODIFY / REMOVE', { tr: 'order_send işlemleri', en: 'order_send actions' }],
  ['ORDER_FILLING_FOK / IOC / RETURN', { tr: 'Doldurma politikası (symbol_info().filling_mode\'a bak)', en: 'Filling policy (see symbol_info().filling_mode)' }],
  ['ORDER_TIME_GTC / DAY / SPECIFIED', { tr: 'Emir geçerlilik süresi', en: 'Order lifetime' }],
  ['POSITION_TYPE_BUY / SELL', { tr: 'Pozisyon yönü', en: 'Position direction' }],
  ['TRADE_RETCODE_DONE (10009)', { tr: 'Başarılı order_send sonucu', en: 'Successful order_send result' }],
  ['COPY_TICKS_ALL / INFO / TRADE', { tr: 'Tick sorgu bayrakları', en: 'Tick query flags' }],
  ['ACCOUNT_TRADE_MODE_DEMO / CONTEST / REAL', { tr: 'Hesap türü (account_info().trade_mode)', en: 'Account type (account_info().trade_mode)' }],
];
