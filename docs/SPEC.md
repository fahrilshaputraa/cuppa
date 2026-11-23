# Spec POS kafe self-hosted PWA

Nov 23, 2026 · @Fahril Shaputra

Tab ini adalah ringkasan. Spec lengkap per modul ada di: Fitur per modul, Role dan hak akses, Pengaturan, Menu dan kustomisasi, Keuangan dan rumus, dan Operasional dan pembayaran.

## Scope

Produk ini satu kafe per instalasi (single-tenant): owner, kasir, dan pelanggan lewat QR meja, semuanya PWA dengan backend Rust, dan kafe yang mengurus hosting-nya sendiri. Pembagian di bawah ini usulan dari riset sebelumnya dan belum divalidasi ke pemilik kafe.

**Mode alur dan pengguna.** Default wizard adalah "Kasir saja": penggunanya owner, kasir, dan pelanggan. Mode "Sampai dapur" menambah pengguna dapur/bar dengan tiket atau layar dapur. Pesanan QR bisa dinyalakan di kedua mode.

**Bahasa.** Antarmuka berbahasa Indonesia dengan terjemahan otomatis Indonesia dan Inggris. Dokumen perancangan ini berbahasa Indonesia. Kode, komentar kode, dan dokumentasi repositori ditulis dalam bahasa Inggris saat dirilis.

**Printer.** Printer ESC/POS yang populer di Indonesia disambungkan lewat LAN. Sumber daya cetak (templat struk, perintah ESC/POS, laci kas) disediakan sebagai pustaka, sehingga perangkat lain bisa disambungkan sesuai pilihan pengguna tanpa mengubah inti aplikasi.

**Pajak.** Tarif, ambang omzet, dan dasar pajak diisi owner sesuai peraturan daerahnya masing-masing.

| Peran | Masuk cakupan |
| --- | --- |
| Pelanggan (web, tanpa install) | Scan QR meja, lihat menu, pilih varian dan tambahan, kirim pesanan, bayar di kasir, QRIS online otomatis bila owner mengaktifkan gateway, rating, loyalty (data pelanggan opsional) |
| Kasir (PWA tablet Android atau PC) | Antrean gabungan pesanan QR dan input manual, bayar tunai atau QRIS (dicatat kasir, atau otomatis lewat payment gateway bila owner punya akun), cetak struk, split bill, gabung dan pecah meja |
| Dapur/bar | Opsional, hanya di alur Sampai dapur: tiket cetak atau layar PWA, plus bunyi notifikasi dan layar dapur lanjutan |
| Owner | Kelola menu, meja dan QR, tarif pajak dan service charge, laporan penjualan, buka/tutup shift, stok bahan dan resep, multi-cabang, integrasi GoFood/GrabFood/ShopeeFood |
| Keamanan | PIN supervisor untuk void dan diskon, log audit, transaksi terkunci setelah dibayar, analitik lanjutan, deteksi anomali |

## Peran dan alur utama

Pelanggan memesan lewat QR, kasir mengonfirmasi, dapur mengerjakan, dan pembayaran selalu di kasir kecuali payment gateway diaktifkan. Konfirmasi kasir adalah usulan default supaya pesanan iseng dari luar kafe tidak langsung masuk dapur.

&#91;embedded content: alur pesanan QR · 8 langkah, 1 keputusan\]

Pesanan yang diinput kasir sendiri langsung masuk dapur tanpa konfirmasi. Pesanan ditolak kembali ke pelanggan dengan status dibatalkan.

| Peran | Perangkat | Tugas utama |
| --- | --- | --- |
| Pelanggan | HP sendiri, browser biasa | Scan QR, pesan, lihat status |
| Kasir | Tablet Android atau PC (Chrome, Edge) | Konfirmasi, input manual, terima bayar, cetak struk, kelola shift |
| Dapur/bar | Printer tiket atau layar PWA | Kerjakan tiket, tandai siap |
| Owner | Laptop atau HP | Atur menu, meja, pajak, lihat laporan |

## Model data

Inti sistem butuh sekitar 12 tabel, dan tiga aturan menentukan sisanya: uang disimpan sebagai bilangan bulat rupiah, harga disalin ke baris pesanan saat dipesan, dan log audit hanya boleh ditambah. Semua ini usulan desain, bukan hasil riset.

| Tabel | Isi utama |
| --- | --- |
| outlet\_settings | Nama kafe, tarif pajak (%), service charge (%), harga inklusif atau eksklusif pajak, service masuk dasar pajak atau tidak |
| users | Nama, peran (owner, kasir, dapur), hash PIN |
| categories, menu\_items | Nama, harga, aktif, tersedia (untuk menu habis) |
| modifier\_groups, modifiers | Varian dan tambahan per menu, harga tambahan, wajib atau opsional |
| tables | Nomor meja, token QR unik, aktif |
| orders | Meja, sumber (qr atau kasir), status, kasir, waktu dibuat dan dibayar |
| order\_items | Menu, jumlah, harga dan modifier yang disalin saat dipesan, catatan |
| payments | Pesanan, metode (tunai, QRIS, lain), jumlah, kasir, waktu |
| shifts | Pembuka, modal awal laci, waktu tutup, uang fisik dihitung, selisih |
| audit\_log | Pelaku, aksi, entitas, nilai sebelum dan sesudah, alasan, waktu |

**Status pesanan:** menunggu konfirmasi, diproses, siap, dibayar, dibatalkan.

**Aturan penting:**

- Harga di `order_items` adalah salinan, jadi mengubah harga menu tidak mengubah riwayat transaksi.
- Uang disimpan sebagai integer rupiah, tanpa desimal.
- Setelah status dibayar, pesanan dan item tidak bisa diubah. Koreksi lewat void atau refund bersama alasan dan PIN supervisor.
- `audit_log` hanya menerima insert, tidak ada update atau delete dari aplikasi.
- Menu tidak dihapus permanen, hanya dinonaktifkan, supaya laporan lama tetap utuh.

**Tabel tambahan untuk fitur penuh** (usulan):

| Tabel | Isi |
| --- | --- |
| role\_permissions, user\_permission\_overrides | Izin per role dan izin tambahan atau dicabut per user |
| devices | Perangkat terdaftar, jenis, terakhir aktif, status cabut |
| printers, stations | Printer, sambungan, lebar kertas, stasiun dan kategori menu yang dilayani |
| table\_sessions | Sesi meja: dibuka, ditutup, pesanan di dalamnya |
| payment\_methods | Metode bayar, aktif, biaya, konfigurasi. Tambahan: payment\_gateway\_configs (kunci API terenkripsi), refunds dan refund\_items (refund sebagai dokumen terpisah), outlets (cabang) |
| opening\_hours, holidays | Jam buka per hari dan hari libur |
| themes, custom\_code\_versions | Token tema, versi kode kustom (teks, hash, penulis, aktif) |
| ingredients, recipes, recipe\_items | Bahan, satuan, resep per menu dan varian |
| suppliers, purchases, purchase\_items, stock\_movements | Pembelian dan semua pergerakan stok (jual, beli, opname, buang) |
| expenses | Biaya operasional: kategori, tanggal, nominal, tetap atau variabel, bukti |
| promotions, vouchers, customers | Promo, voucher, data pelanggan |

## Database: SQLite default, Postgres opsional

SQLite adalah default karena cukup untuk satu kafe dan cuma berupa satu file, sedangkan Postgres dipilih lewat pengaturan untuk yang punya server database atau banyak cabang. Mendukung dua database sekaligus menambah pekerjaan, jadi disiplin berikut perlu dijaga sejak awal. Semua poin teknis di bagian ini pengetahuan umum, bukan hasil riset.

| Hal | SQLite (default) | Postgres (opsional) |
| --- | --- | --- |
| Pemasangan | Satu file, tanpa server database | Butuh server Postgres dan akun koneksi |
| Backup | Salin file memakai mekanisme backup yang aman saat database sedang dipakai, bukan menyalin file begitu saja | Dump database terjadwal |
| Penulisan bersamaan | Satu penulis dalam satu waktu. Mode WAL membuat pembaca tidak terblokir. Cukup untuk satu kafe dengan beberapa perangkat | Banyak penulis bersamaan |
| Cocok untuk | Satu kafe, server di kafe | Multi-cabang, server pusat, banyak tenant di versi cloud |
| Realtime ke layar | Siaran dalam proses server | Siaran dalam proses server, karena aplikasi tetap satu proses |

**Aturan supaya dua database tetap bisa didukung:**

1. Variabel `DATABASE_URL` memilih database, dengan default file SQLite di folder data.
2. Satu lapisan akses data yang bisa memakai kedua database. Pustaka seperti SeaORM memilih backend saat program berjalan, sedangkan SQLx dengan makro pemeriksa query memeriksa terhadap satu database saat kompilasi. Pilihan pustaka perlu diputuskan sebelum menulis query pertama.
3. Migrasi ditulis per database, dan seluruh tes otomatis dijalankan di keduanya.
4. Hindari fitur khusus satu database (array dan operator JSON Postgres, row-level security). Uang disimpan sebagai integer, waktu dalam UTC, id sebagai teks (UUID atau ULID).
5. Nomor pesanan harian dibuat di dalam transaksi supaya tidak ada nomor ganda. Di SQLite artinya transaksi yang langsung mengunci penulisan.
6. Alat bawaan untuk memindahkan data dari SQLite ke Postgres tersedia, karena kafe yang tumbuh atau menambah cabang akan membutuhkannya.

## Endpoint API

API dibagi empat kelompok menurut siapa yang memanggil, dan satu aliran realtime untuk notifikasi pesanan baru. Stack yang diusulkan: Axum, SQLx, SQLite mode WAL sebagai default (Postgres opsional), dan Server-Sent Events untuk realtime karena arahnya satu sisi dari server ke layar kasir dan dapur. Nama path di bawah contoh, bukan keputusan final.

| Kelompok | Endpoint | Fungsi |
| --- | --- | --- |
| Pelanggan (token meja di URL) | `GET /api/m/{token}/menu` | Menu aktif beserta modifier |
|  | `POST /api/m/{token}/orders` | Kirim pesanan, status awal menunggu konfirmasi |
|  | `GET /api/m/{token}/orders/{id}` | Cek status pesanan |
| Auth | `POST /api/auth/pin` | Login staf dengan PIN, mengembalikan sesi |
| Kasir | `GET /api/orders?status=` | Antrean pesanan |
|  | `POST /api/orders` | Input pesanan manual |
|  | `PATCH /api/orders/{id}/confirm` | Konfirmasi pesanan QR, lanjut ke dapur |
|  | `POST /api/orders/{id}/pay` | Catat pembayaran dan kunci pesanan |
|  | `POST /api/orders/{id}/void` | Batalkan, wajib PIN supervisor dan alasan |
|  | `POST /api/orders/{id}/print` | Cetak ulang struk atau tiket |
|  | `POST /api/shifts/open`, `POST /api/shifts/close` | Buka dan tutup shift, hitung kas fisik |
| Dapur/bar | `GET /api/kitchen/tickets` | Tiket yang sedang berjalan |
|  | `PATCH /api/kitchen/tickets/{id}` | Tandai diproses atau siap |
| Owner | `/api/menu`, `/api/modifiers`, `/api/tables` | CRUD menu, modifier, meja, dan generate QR |
|  | `GET/PUT /api/settings` | Pajak, service charge, nama kafe |
|  | `GET /api/reports/sales?from=&to=` | Penjualan per hari, per menu, per kasir, void dan diskon |
| Realtime | `GET /api/events` (SSE) | Pesanan baru, perubahan status, bunyi notifikasi |

**Token meja:** acak panjang per meja, bisa diputar ulang oleh owner. Pembatasan jumlah request per token dan konfirmasi kasir sebelum pesanan masuk dapur mengurangi pesanan iseng dari luar kafe. Ini usulan, belum diuji.

## Cetak, hardware, dan offline

Cara mencetak ditentukan oleh letak server: server di dalam jaringan kafe bisa menembak printer langsung, sedangkan server di VPS butuh printer yang menarik tugas cetak atau agen kecil di kafe. Karena itu keputusan letak server harus diambil sebelum menulis modul cetak.

| Letak server | Cara cetak | Catatan |
| --- | --- | --- |
| PC atau mini PC di kafe | Server mengirim perintah ESC/POS langsung ke printer LAN (umumnya port 9100) | Paling sederhana, tetap jalan saat internet putus. Port 9100 dari pengetahuan umum, belum diverifikasi |
| VPS di internet | Printer menarik tugas dari server: Star CloudPRNT atau Epson Server Direct Print | Dari riset: printer yang memanggil server Anda, jadi tanpa browser dan tanpa agen lokal. Perlu printer yang mendukung |
| VPS di internet | Agen cetak kecil di kafe | Satu program tambahan yang harus dipasang dan dirawat |

**Mengapa tidak cetak dari browser:** Web Bluetooth hanya ada di Chromium dan tidak tersedia di iOS. Web Serial baru ada di Chrome Android sejak versi 148 (Mei 2026) dan ditolak Safari. WebUSB belum standar dan butuh izin pengguna. Maka iPad tidak bisa mencetak langsung dari browser, dan target kasir adalah tablet Android atau PC dengan Chrome atau Edge.

**Laci kas:** biasanya dibuka lewat printer (kabel RJ11 ke printer, printer mengirim pulsa buka). Ini pengetahuan umum, belum diriset.

**Harga acuan:** satu listing Tokopedia menunjukkan Epson TM-T82X USB Rp2.699.000. Harga printer LAN murah (Xprinter dan sejenisnya) tidak ditemukan di riset, perlu dicek langsung.

**Offline:** server berjalan di jaringan kafe sehingga kasir dan dapur tetap jalan tanpa internet. PWA menyimpan cache aplikasi lewat service worker. Mode offline penuh dengan sinkronisasi antarperangkat (FR-63 di PRD) adalah modul tersendiri karena rumit: tiap jenis data butuh aturan penyelesaian konflik.

**QR meja dan akses pelanggan:** HP pelanggan harus bisa membuka halaman. Pilihannya domain publik dengan HTTPS di VPS, server di kafe yang diekspos lewat tunnel seperti Cloudflare Tunnel, atau hanya lewat WiFi kafe. Dua opsi terakhir pengetahuan umum, belum diriset.

## Anti-bocor, pajak, dan service charge

Kontrol kebocoran kas dan hitungan pajak harus ada sejak awal karena dua hal itu yang paling sering membuat pemilik kafe mau memakai POS. Empat modus kecurangan kasir yang dicatat vendor IREAP: membatalkan transaksi setelah uang tunai diterima, membatalkan pesanan yang sudah dibayar, membuka kembali bill lalu menghapus item, dan diskon fiktif. Angka kerugiannya hanya ilustrasi vendor.

| Modus | Kontrol di aplikasi |
| --- | --- |
| Void setelah bayar tunai | PIN supervisor dan alasan wajib, transaksi terkunci setelah dibayar |
| Batal pesanan yang sudah dibayar | Hanya lewat void atau refund yang tercatat di log audit |
| Hapus item setelah bayar | Item tidak bisa diubah setelah dibayar |
| Diskon fiktif | Diskon butuh izin, laporan diskon per kasir per shift |
| Uang laci tidak cocok | Penutupan shift buta: kasir menghitung uang fisik sebelum melihat angka sistem |

**Pajak.** PB1 kini menjadi PBJT atas makanan dan minuman: tarif maksimal 10%, ditetapkan perda, dan batas omzet ditentukan pemerintah daerah. Sebagian daerah memakai tarif berjenjang. Karena itu tarif, harga inklusif atau eksklusif pajak, dan apakah service charge masuk dasar pajak harus bisa diatur per instalasi. Tarif dan batas omzet diisi owner sesuai peraturan daerah masing-masing.

**Contoh hitung** (dari Klikpajak, harga eksklusif pajak, service 5% masuk dasar pajak, tarif pajak 10%):

| Komponen | Rupiah |
| --- | --- |
| Makanan | 80.000 |
| Service charge 5% | 4.000 |
| Dasar pajak | 84.000 |
| Pajak 10% | 8.400 |
| Total | 92.400 |

**QRIS.** Sejak 1 Oktober 2026, MDR QRIS 0% sampai Rp100.000 untuk usaha kecil sampai besar dan sampai Rp500.000 untuk usaha mikro. Di atas batas itu tetap 0,7% dan 0,3%. Konsumen tidak boleh dikenai biaya admin QRIS. Kasir cukup mencatat metode bayar QRIS. Konfirmasi otomatis lewat gateway bersifat opsional.

## Urutan build dan deployment

Bangun dari alur uang dulu (pesanan, bayar, shift), baru ke cetak dan kemasan, dan uji di kafe nyata. Urutan ini usulan.

1. **Dasar:** proyek Axum dan SQLx, migrasi database, login PIN dengan peran, log audit.
2. **Master data:** menu, modifier, meja, token QR dan halaman cetak QR.
3. **Pesanan:** halaman pelanggan, antrean kasir, konfirmasi, aliran SSE.
4. **Bayar dan shift:** pembayaran tunai dan QRIS tercatat, kunci transaksi, buka dan tutup shift, void dengan PIN supervisor.
5. **Struk dan dapur:** struk dan tiket dari tampilan cetak dulu, lalu ESC/POS ke printer LAN, layar dapur PWA.
6. **Pajak dan laporan:** tarif dapat diatur, laporan penjualan, void dan diskon per kasir.
7. **Kemasan:** satu binary atau Docker, panduan instalasi langkah demi langkah, backup database harian, prosedur update yang aman.
8. **Uji di kafe nyata:** pasang di satu kafe, catat semua masalah, dan perbaiki sebelum rilis.

**Deployment self-hosted:** satu binary Rust dengan file SQLite, di belakang nginx sebagai reverse proxy, dengan HTTPS dari sertifikat Let's Encrypt lewat certbot (pengetahuan umum, belum diriset). Backup cukup menyalin file database ke tempat lain secara terjadwal. Langkah 7 sering menjadi sumber tiket support terbesar, jadi pasang di mesin bersih dan catat setiap kendala.

**Konfigurasi nginx.** Aplikasi Rust mendengarkan di alamat lokal (misalnya port 8080) dan hanya nginx yang terbuka ke internet. Tiga hal yang mudah terlewat: aliran realtime (SSE) tidak boleh di-buffer, batas unggahan harus cukup untuk foto menu, dan endpoint pelanggan perlu pembatasan request. Contoh di bawah belum diuji di server nyata, jadi jalankan `nginx -t` dan sesuaikan nama domain, port, dan jalur sertifikat.

```nginx
# di blok http (nginx.conf)
limit_req_zone $binary_remote_addr zone=pelanggan:10m rate=5r/s;

server {
    listen 80;
    server_name kafe.contoh.id;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    server_name kafe.contoh.id;

    ssl_certificate     /etc/letsencrypt/live/kafe.contoh.id/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/kafe.contoh.id/privkey.pem;

    client_max_body_size 5m;   # unggah foto menu

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # halaman dan API pelanggan: batasi request per IP
    location /api/m/ {
        limit_req zone=pelanggan burst=10 nodelay;
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # aliran realtime (SSE): jangan di-buffer
    location /api/events {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 1h;
    }
}
```

- Aplikasi hanya boleh memercayai header `X-Forwarded-For` jika permintaan datang dari nginx, supaya pembatasan per IP tidak bisa dipalsukan.
- Sertifikat dibuat dan diperpanjang otomatis oleh certbot. Server harus bisa dijangkau dari internet di port 80 dan 443 agar pengecekan domain berhasil.
- Origin terpisah untuk kode kustom (tab Menu dan kustomisasi) butuh blok `server` dan sertifikat sendiri, misalnya subdomain `kustom.kafe.contoh.id`, dengan header CSP yang ketat.
- Jika server berada di PC kafe tanpa alamat publik, nginx tetap bisa dipakai di PC itu dengan Cloudflare Tunnel di depannya. Ini pengetahuan umum, belum diriset.
