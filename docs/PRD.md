# PRD POS kafe self-hosted dengan pesanan QR

Nov 23, 2026 · @Fahril Shaputra

## Ringkasan

Produk ini adalah aplikasi kasir (POS) dan pesanan QR berbasis web (PWA) untuk kafe dan restoran kecil di Indonesia, dengan backend Rust, yang dipasang dan dijalankan sendiri oleh pemilik kafe (self-hosted). Status dokumen: draf 0.1, dan semua prioritas di dalamnya belum divalidasi ke pemilik kafe.

**Masalah.** Pemilik kafe kecil kehilangan uang di tiga tempat: kebocoran kas (void dan diskon fiktif, selisih laci), kecepatan kasir saat ramai, dan pencatatan laba yang masih manual. Produk POS yang ada berharga sekitar Rp150 ribu sampai Rp1 juta per outlet per bulan, dan QR order di salah satu yang terbesar baru tersedia di paket Rp499 ribu. Dasar angka dan sumbernya ada di dokumen [Spec](https://claude.ai/artifact/Y7EUBnavraMkWiMtWPVpSg) dan riset sebelumnya.

**Solusi.** POS yang cukup sampai kasir untuk kafe yang baru merintis, yang bisa dinaikkan ke pesanan QR di meja dan tiket dapur lewat pengaturan, dengan kontrol kebocoran bawaan, laporan laba berumus jelas, tampilan menu yang bisa dikustomisasi dengan aman, dan database SQLite sebagai default (Postgres opsional).

**Untuk siapa.** Pengguna utama adalah pemilik kafe atau restoran kecil yang baru merintis. Pengguna sehari-hari adalah kasir dan pelanggan. Pemasangan dan pemeliharaan sistem dilakukan oleh pemilik atau orang yang dibantunya, sehingga pemasangan harus bisa diikuti orang non-teknis.

## Tujuan, non-tujuan, dan metrik keberhasilan

Tujuan produk ini adalah satu kafe nyata memakai produk ini setiap hari selama empat minggu tanpa kehilangan data dan tanpa celah kebocoran kas yang mudah. Angka target di bawah adalah usulan karena belum ada data baseline, dan akan dikoreksi setelah uji di kafe nyata.

**Tujuan:**

- Kasir menyelesaikan transaksi dengan cepat dan tanpa pelatihan panjang.
- Setiap void, diskon, dan refund tercatat dengan pelaku dan penyetuju.
- Owner melihat penjualan, kontrol, dan laba dari rumus yang bisa diperiksa dengan kalkulator.
- Orang non-teknis bisa memasang aplikasi dengan panduan.
- Satu aplikasi melayani kafe yang hanya butuh kasir sampai yang memakai QR dan dapur, lewat pengaturan.

**Non-tujuan:**

- Menerima atau menahan uang pelanggan. Produk bukan penyedia pembayaran.
- Penjualan perangkat keras. Produk hanya perangkat lunak.

| Metrik | Target (usulan) | Cara mengukur |
| --- | --- | --- |
| Waktu transaksi tunai sederhana (tiga item) | 30 detik atau kurang | Diukur dengan stopwatch di kafe nyata |
| Waktu pemasangan oleh orang non-teknis dengan panduan | 30 menit atau kurang | Uji ke tiga orang |
| Transaksi terbayar yang berubah setelah dibayar | Nol | Tes otomatis dan log audit |
| Void, diskon, dan refund yang tercatat dengan penyetuju | Seratus persen | Tes otomatis dan pemeriksaan log |
| Uji di satu kafe nyata | Empat minggu pemakaian harian tanpa kehilangan data | Catatan masalah selama uji |
| Uji pulih backup | Berhasil setiap kali diuji | Uji terjadwal bulanan |
| Porsi pesanan makan di tempat yang masuk lewat QR | Ditetapkan setelah uji di kafe nyata, karena tidak ada data baseline | Laporan sumber pesanan |

## Pengguna dan persona

Pengguna utama adalah pemilik kafe kecil yang baru merintis dan belum memakai POS, dengan kasir dan pelanggan sebagai pengguna harian. Semua persona di bawah hipotesis dari riset di internet dan belum diuji lewat wawancara, jadi dasarnya lemah sampai wawancara dilakukan.

| Persona | Konteks | Kebutuhan utama | Kekhawatiran |
| --- | --- | --- | --- |
| Pemilik kafe merintis | Satu outlet dan satu sampai tiga staf, sering ikut menjaga kasir, belum punya POS, tidak mengerti server | Tahu uang masuk hari ini, mencegah kebocoran, laporan laba yang mudah dibaca, pemasangan yang tidak ribet | Biaya bulanan, data hilang, aplikasi rumit, ketergantungan pada satu developer |
| Kasir atau barista yang merangkap | Melayani pelanggan sambil membuat minuman, tablet Android atau PC | Input cepat dengan sedikit ketukan, tidak macet saat ramai, tidak disalahkan atas selisih kas yang bukan salahnya | Salah memencet, antre panjang, harus mengingat banyak aturan |
| Pelanggan | Memakai HP sendiri, tidak mau memasang aplikasi | Memesan atau membayar dengan cepat, melihat harga yang jelas termasuk pajak | Harga di layar berbeda dari tagihan, kehilangan pesanan |
| Pemilik yang berkembang | Dua cabang atau lebih, ingin melihat semuanya dari satu tempat | Laporan gabungan dan perbandingan cabang, pajak dan harga per cabang | Data tercampur antar cabang, server sulit dijangkau |
| Pemasang atau pengelola sistem | Memasang, memperbarui, dan memelihara sistem kafe | Panduan pemasangan, pembaruan yang aman, backup yang bisa dipulihkan, log untuk mencari masalah | Masalah yang menumpuk, pemasangan yang salah |

Survei konsumen di Amerika Serikat menunjukkan sebagian besar pelanggan masih lebih suka menu fisik daripada menu QR (satu persen memilih QR dan 81 persen memilih menu fisik, dari 850 orang dewasa). Data itu tidak bisa disamakan dengan Indonesia, tetapi menjadi alasan produk ini tidak memaksa pelanggan memakai QR dan tetap mendukung pemesanan langsung ke kasir.

## Cerita pengguna

Tujuh belas cerita di bawah mencakup seluruh alur inti dari pemasangan sampai tutup shift, dan tiap cerita punya kriteria penerimaan yang bisa diuji.

| ID | Cerita | Kriteria penerimaan |
| --- | --- | --- |
| US-01 | Sebagai kasir, saya ingin membuat pesanan dengan varian dan catatan agar pesanan sesuai permintaan | Varian wajib harus dipilih, total mengikuti pengaturan pajak dan service, tiga item selesai dalam 30 detik atau kurang |
| US-02 | Sebagai kasir, saya ingin menerima tunai dan melihat kembalian agar tidak salah hitung | Ada tombol uang pas, kembalian benar, laci terbuka, struk tercetak, pesanan terkunci setelah dibayar |
| US-03 | Sebagai kasir, saya ingin mencatat pembayaran QRIS, kartu, atau transfer agar laporan metode bayar benar | Metode dan nomor referensi tersimpan, nomor kartu lengkap tidak pernah disimpan |
| US-04 | Sebagai kasir, saya ingin membatalkan item sebelum dibayar agar pesanan yang salah bisa dikoreksi | Alasan wajib, tercatat di log, stok per menu dikembalikan |
| US-05 | Sebagai manajer atau owner, saya ingin menyetujui void, diskon besar, dan refund dengan PIN agar kebocoran bisa dicegah | Penyetuju adalah orang lain, pelaku dan penyetuju tercatat, PIN salah lima kali mengunci sementara |
| US-06 | Sebagai kasir, saya ingin membuka dan menutup shift dengan hitung uang fisik agar selisih kas terlihat | Penutupan buta: angka sistem muncul setelah hitungan fisik dimasukkan, laporan Z tersedia |
| US-07 | Sebagai pelanggan, saya ingin memindai QR meja dan memesan tanpa memasang aplikasi atau login | Harga dihitung ulang di server, pesanan masuk antrean kasir dengan nomor meja, menu habis ditolak |
| US-08 | Sebagai pelanggan, saya ingin melihat status pesanan agar tahu kapan dilayani | Status mengikuti mode alur yang dipilih owner |
| US-09 | Sebagai kasir, saya ingin mengonfirmasi atau menolak pesanan QR agar pesanan iseng tidak masuk | Bunyi saat pesanan masuk, penolakan memakai alasan, pesanan tanpa konfirmasi batal otomatis setelah N menit |
| US-10 | Sebagai owner, saya ingin mengatur menu, varian, dan harga agar daftar harga selalu terbaru | Mengubah harga tidak mengubah transaksi lama, menu bisa dinonaktifkan tanpa dihapus, impor dari CSV tersedia |
| US-11 | Sebagai owner, saya ingin mengatur pajak dan service charge agar sesuai aturan daerah saya | Tarif, harga inklusif atau eksklusif, dan status service dalam dasar pajak bisa diatur, berlaku hanya untuk pesanan baru |
| US-12 | Sebagai owner, saya ingin melihat penjualan harian dan laporan kontrol agar tahu hasil dan kebocoran | Penjualan per hari, per menu, per kasir, per metode bayar, dan laporan void, diskon, serta selisih kas |
| US-13 | Sebagai owner, saya ingin memilih apakah memakai pesanan QR dan tiket dapur agar aplikasi pas dengan cara kerja kafe saya | Empat kombinasi didukung, halaman yang tidak dipakai tidak tampil, wizard awal menanyakan dua pilihan itu |
| US-14 | Sebagai owner, saya ingin mengelola user dan role agar akses sesuai tugas | Izin per role dapat ditambah atau dicabut per user, user keluar dinonaktifkan bukan dihapus |
| US-15 | Sebagai pemasang, saya ingin memasang aplikasi dari panduan dan memulihkan backup agar data aman | Pemasangan 30 menit atau kurang, uji pulih backup berhasil |
| US-16 | Sebagai staf dapur (mode Sampai dapur), saya ingin melihat tiket dan menandai siap agar pesanan keluar berurutan | Tiket per stasiun, bunyi saat tiket baru, status berubah di layar kasir |
| US-17 | Sebagai kasir, saya ingin setiap pesanan konter punya nomor antrean agar pelanggan bisa dipanggil | Nomor urut per hari, tercetak di struk, tanpa nomor ganda |

## Persyaratan fungsional inti

FR-01 sampai FR-40 adalah persyaratan inti, sebagian besar wajib, dan sebagian hanya berlaku jika owner menyalakan pesanan QR atau tiket dapur. Prioritas: Wajib berarti rilis tidak boleh keluar tanpa itu, Sebaiknya berarti dikerjakan jika waktu cukup. Rincian per layar dan per field ada di dokumen [Spec](https://claude.ai/artifact/Y7EUBnavraMkWiMtWPVpSg).

**Pelanggan (hanya jika pesanan QR menyala)**

| ID | Persyaratan | Prioritas | Cerita |
| --- | --- | --- | --- |
| FR-01 | Halaman dibuka dari URL dengan token meja, tanpa login, menampilkan nama kafe dan status buka atau tutup | Wajib | US-07 |
| FR-02 | Menu per kategori dengan foto, harga, label, dan tanda habis | Wajib | US-07 |
| FR-03 | Pilih varian dan tambahan sesuai aturan wajib dan batas pilihan | Wajib | US-07 |
| FR-04 | Keranjang dengan catatan dan rincian subtotal, service, pajak, dan total sesuai pengaturan | Wajib | US-07 |
| FR-05 | Server menghitung ulang harga dan menolak menu habis atau nonaktif, meja tidak aktif, dan permintaan di luar jam buka | Wajib | US-07 |
| FR-06 | Halaman status pesanan sesuai mode alur | Wajib | US-08 |
| FR-07 | Menambah pesanan pada sesi meja yang sama | Sebaiknya | US-07 |

**Kasir**

| ID | Persyaratan | Prioritas | Cerita |
| --- | --- | --- | --- |
| FR-08 | Antrean gabungan pesanan QR dan manual, dengan bunyi saat pesanan masuk | Wajib | US-09 |
| FR-09 | Konfirmasi atau tolak pesanan QR dengan alasan, dan batal otomatis jika tidak dikonfirmasi dalam N menit | Wajib | US-09 |
| FR-10 | Input pesanan manual dengan tipe pesanan, meja opsional, dan nomor antrean urut harian | Wajib | US-01, US-17 |
| FR-11 | Edit item sebelum dibayar dan pindah meja | Wajib | US-01 |
| FR-12 | Diskon per item atau pesanan dengan batas per role | Wajib | US-05 |
| FR-13 | Pembayaran tunai (kembalian, uang pas), QRIS statis, kartu EDC, dan transfer dengan nomor referensi | Wajib | US-02, US-03 |
| FR-14 | Cetak struk dan cetak ulang yang tercatat di log | Wajib | US-02 |
| FR-15 | Void sebelum bayar dengan alasan | Wajib | US-04 |
| FR-16 | Refund penuh atau sebagian sebagai dokumen terpisah, dengan PIN supervisor dan alasan | Wajib | US-05 |
| FR-17 | Kas masuk, kas keluar, dan buka laci tanpa transaksi dengan izin dan alasan | Sebaiknya | US-06 |
| FR-18 | Buka dan tutup shift dengan penutupan buta dan laporan Z | Wajib | US-06 |
| FR-19 | Riwayat dan pencarian pesanan | Wajib | US-01 |

**Dapur dan owner**

| ID | Persyaratan | Prioritas | Cerita |
| --- | --- | --- | --- |
| FR-20 | Pemetaan kategori ke stasiun, printer, atau layar, hanya di mode Sampai dapur | Wajib jika mode aktif | US-16 |
| FR-21 | Tiket cetak atau layar PWA dengan bunyi dan perubahan status | Wajib jika mode aktif | US-16 |
| FR-22 | Menandai menu habis oleh kasir atau dapur | Wajib | US-07 |
| FR-23 | CRUD menu, kategori, modifier, dan foto, serta impor CSV | Wajib | US-10 |
| FR-24 | Stok per menu opsional dengan otomatis habis di angka nol | Sebaiknya | US-10 |
| FR-25 | CRUD meja, cetak QR, dan putar ulang token, hanya jika pesanan QR menyala | Wajib jika mode aktif | US-07 |
| FR-26 | Pengaturan pajak, service, struk, pembulatan, dan jam buka | Wajib | US-11 |
| FR-27 | Wizard awal dan pilihan empat kombinasi alur | Wajib | US-13 |
| FR-28 | Dashboard dan laporan penjualan, kontrol, shift, dan pajak | Wajib | US-12 |
| FR-29 | Ekspor CSV laporan | Sebaiknya | US-12 |
| FR-30 | Tema halaman pelanggan berbasis token (warna, font, sudut) | Sebaiknya | US-10 |

**Keamanan dan sistem**

| ID | Persyaratan | Prioritas | Cerita |
| --- | --- | --- | --- |
| FR-31 | Role bawaan dan matriks izin, dengan izin tambahan atau dicabut per user, diperiksa server di setiap endpoint | Wajib | US-14 |
| FR-32 | Login PIN untuk staf di perangkat terdaftar, dan kata sandi untuk owner dan manajer | Wajib | US-14 |
| FR-33 | PIN supervisor dengan penyetuju orang lain, kunci setelah lima kali salah, dan kunci layar saat diam | Wajib | US-05 |
| FR-34 | Log audit yang hanya bisa ditambah, dan transaksi terkunci setelah dibayar | Wajib | US-05 |
| FR-35 | SQLite sebagai default | Wajib | US-15 |
| FR-36 | Postgres sebagai opsi lewat variabel `DATABASE_URL` | Sebaiknya | US-15 |
| FR-37 | Backup manual, unduh, dan uji pulih | Wajib | US-15 |
| FR-38 | Pembaruan aplikasi dengan migrasi database yang aman | Wajib | US-15 |
| FR-39 | Pencetakan ke printer LAN (ESC/POS), buka laci lewat printer, dan tombol uji cetak | Wajib | US-02 |
| FR-40 | Paket pemasangan (binary atau Docker), panduan langkah demi langkah, dan contoh konfigurasi nginx | Wajib | US-15 |

## Persyaratan fungsional tambahan

Persyaratan di bawah melengkapi FR-01 sampai FR-40 dan termasuk dalam cakupan produk. Urutan pengerjaan ditentukan di rencana teknis, bukan di dokumen ini.

| ID | Persyaratan | Catatan |
| --- | --- | --- |
| FR-41 | Payment gateway opsional (Midtrans, Xendit, dan sejenisnya): QRIS dinamis, webhook idempoten, rekonsiliasi pembayaran menunggu | Butuh URL publik HTTPS. Aplikasi tidak memegang uang |
| FR-42 | Refund lewat gateway dengan status menunggu, berhasil, atau gagal, dan jalur pengembalian manual | Aturan refund berbeda per penyedia dan penerbit QRIS |
| FR-43 | Pembayaran campuran dan split bill |  |
| FR-44 | Stok bahan, resep, pembelian, stock opname, pemborosan, dan HPP aktual | Melengkapi stok per menu |
| FR-45 | Halaman Biaya dan laporan laba: HPP, biaya operasional, laba bersih, titik impas | Rumus ada di tab Keuangan dan rumus pada Spec |
| FR-46 | Multi-cabang: role per cabang, menu bersama dengan harga per cabang, laporan gabungan dan perbandingan | Kolom outlet\_id ada di semua tabel. Praktisnya butuh server pusat |
| FR-47 | Rating pesanan dan rating per cabang | Satu rating per sesi meja |
| FR-48 | Statistik lanjutan: margin per menu, perubahan peringkat, peta panas jam sibuk, mode sibuk otomatis | Mode sibuk otomatis hanya di mode Sampai dapur |
| FR-49 | Promo, voucher, dan harga terjadwal |  |
| FR-50 | CSS kustom dan blok HTML kustom yang disaring | Perlu tinjauan keamanan sebelum rilis |
| FR-51 | Backup terjadwal, autentikasi dua langkah untuk owner, dan role kustom |  |
| FR-52 | Struk digital lewat tautan dan pengiriman lewat WhatsApp |  |
| FR-53 | Denah meja, gabung dan pecah meja, panggil pelayan, minta bill, estimasi waktu, dan terjemahan otomatis Indonesia dan Inggris |  |
| FR-54 | Alat pemindahan data dari SQLite ke Postgres |  |
| FR-55 | Pesanan dari ojek online atau pesan antar yang diinput manual dengan sumber penjualan | Belum integrasi otomatis |
| FR-56 | JS kustom di iframe sandbox pada origin terpisah | Dasar keamanan di tab Menu dan kustomisasi pada Spec |
| FR-57 | Data pelanggan opsional, total pembelian per pelanggan, menu favorit pelanggan, loyalty | Data pribadi, aturan pelindungan data perlu dicek |
| FR-58 | Rating per menu |  |
| FR-59 | Integrasi GoFood, GrabFood, ShopeeFood, dan akuntansi | Syarat kerja sama platform belum diriset |
| FR-60 | Transfer stok antar cabang dan produksi bahan setengah jadi |  |
| FR-61 | Reservasi dan uang muka |  |
| FR-62 | Layar antrean untuk pelanggan, rekomendasi jadwal staf, API dan webhook keluar |  |
| FR-63 | Mode offline penuh: kasir tetap bisa bertransaksi saat server tidak terjangkau, dengan sinkronisasi antarperangkat dan penyelesaian konflik setelah tersambung | Melengkapi cache PWA. Aturan konflik per jenis data ditetapkan di Spec |
| FR-64 | Aplikasi native pembungkus PWA untuk Android, iOS, dan desktop, termasuk akses langsung ke printer dan laci kas | Mengatasi batas browser untuk cetak langsung di iPad |
| FR-65 | Pencatatan akuntansi dasar (jurnal dan buku besar) dan ekspor laporan pajak daerah dari data penjualan | Format laporan pajak mengikuti ketentuan daerah owner |
| FR-66 | Mode multi-tenant untuk dijalankan sebagai layanan cloud: satu instalasi melayani banyak kafe dengan data terpisah | Memakai Postgres. Isolasi data per tenant wajib diuji |

## Persyaratan non-fungsional

Tiga persyaratan non-fungsional paling menentukan: integritas data uang, keamanan akses dan kode kustom, dan kemampuan kasir tetap bekerja saat internet putus. Angka target di tabel adalah usulan dan harus diukur di perangkat nyata sebelum dijadikan janji ke pengguna.

| Kategori | Persyaratan | Target atau ukuran |
| --- | --- | --- |
| Kinerja | Respons API untuk operasi kasir | 300 ms atau kurang pada persentil 95, dengan beban satu kafe: hingga lima perangkat bersamaan dan 60 pesanan per jam |
| Kinerja | Halaman menu pelanggan di jaringan seluler biasa | Terbuka dalam tiga detik atau kurang dengan ukuran awal sekitar 300 KB (foto dimuat bertahap) |
| Kinerja | Notifikasi pesanan baru ke layar kasir dan dapur | Dua detik atau kurang lewat aliran SSE |
| Ketersediaan | Server di kafe tetap melayani kasir dan dapur tanpa internet | Semua alur inti kecuali fitur yang memerlukan internet (gateway, pesanan dari luar jaringan kafe) |
| Ketersediaan | Pemulihan setelah mati listrik atau restart | Aplikasi jalan otomatis saat server hidup, tanpa kehilangan transaksi yang sudah tersimpan |
| Integritas data | Uang dan angka transaksi | Bilangan bulat rupiah, tidak ada pembulatan diam-diam, pembulatan tercatat sebagai baris sendiri |
| Integritas data | Nomor pesanan | Tidak ada nomor ganda atau lompatan, dibuat dalam transaksi database |
| Integritas data | Migrasi dan pembaruan | Tiap versi baru bisa dipasang tanpa merusak data, dengan backup otomatis sebelum migrasi |
| Keamanan | Otorisasi | Setiap endpoint memeriksa izin di server, bukan hanya menyembunyikan tombol |
| Keamanan | PIN dan kata sandi | Disimpan sebagai hash, pembatasan percobaan, sesi di cookie HttpOnly dan SameSite |
| Keamanan | Data pembayaran | Nomor kartu lengkap tidak pernah disimpan, kunci API gateway disimpan terenkripsi |
| Keamanan | Unggahan | Hanya gambar, diubah ukuran dan disimpan ulang di server, nama file acak |
| Keamanan | Kode buatan pengguna | Server tidak pernah mengeksekusinya, dan JS hanya di iframe sandbox. Perlu tinjauan ahli keamanan sebelum dirilis |
| Keamanan | Jaringan | HTTPS wajib untuk akses dari internet, pembatasan request per IP pada endpoint pelanggan |
| Privasi | Data pelanggan | Hanya dikumpulkan jika owner menyalakannya dan pelanggan menyetujui. Aturan pelindungan data pribadi di Indonesia belum diriset dan harus dicek sebelum fitur ini dipakai |
| Kompatibilitas | Perangkat kasir dan dapur | Chrome atau Edge terbaru di Android dan PC. iPad dan Safari bukan target kasir karena API perangkat tidak didukung |
| Kompatibilitas | Perangkat pelanggan | Browser modern di Android dan iOS, tanpa instalasi |
| Kompatibilitas | Spesifikasi tablet kasir | Acuan: tablet delapan inci, RAM 4 GB, Android 7 atau lebih baru, mengikuti spesifikasi yang disebut vendor POS lain |
| Kompatibilitas | Printer | Printer termal ESC/POS 58 atau 80 mm lewat jaringan LAN, laci kas lewat printer |
| Lokalisasi | Bahasa, mata uang, waktu | Bahasa Indonesia, rupiah tanpa desimal, zona waktu per outlet dengan default Asia/Jakarta |
| Kegunaan | Layar kasir | Target sentuh besar (usulan 44 piksel), teks terbaca di tablet, bekerja dengan satu tangan |
| Operasional | Pemantauan dan dukungan | Endpoint cek kesehatan, log aplikasi dengan rotasi, nomor versi terlihat di pengaturan |

## Platform dan arsitektur ringkas

Detail teknis lengkap ada di dokumen "Spec POS kafe self-hosted PWA". Bagian ini hanya keputusan yang membatasi produk.

| Area | Keputusan | Alasan dan batasan |
| --- | --- | --- |
| Backend | Rust dengan Axum dan SQLx, satu binary | Mudah dipasang di server kafe, tanpa runtime tambahan |
| Database | SQLite mode WAL secara default, Postgres opsional lewat `DATABASE_URL` | SQLite cukup untuk satu kafe. Multi-cabang praktis butuh server pusat dan Postgres. Migrasi dan tes harus berjalan di kedua dialek |
| Antarmuka | HTML dari server (Askama atau Maud) dengan HTMX dan sedikit Alpine, dikirim sebagai PWA | Pilihan pengguna. Satu bahasa dan satu deployment. Pembaruan layar langsung memakai SSE |
| Perangkat kasir | Tablet Android atau PC dengan Chrome atau Edge | iPad dan Safari tidak punya Web Serial, Bluetooth, atau USB, jadi cetak langsung tidak bisa dari sana |
| Cetak struk | LAN ESC/POS (TCP 9100) dari server di kafe. Untuk server di VPS: Star CloudPRNT, Epson Server Direct Print, atau agen lokal | Laci kas dibuka lewat printer |
| Akses pelanggan | QR per meja berisi token di URL, sesi per meja, harga dihitung ulang di server | Token meja bukan rahasia, jadi konfirmasi kasir dan batas laju per token dan IP jadi pengaman |
| Jaringan | Nginx sebagai reverse proxy, HTTPS dengan Let's Encrypt, opsional Cloudflare Tunnel | Konfigurasi nginx di spec belum diuji |
| Kustomisasi | Tema lewat token. CSS, HTML, dan JS buatan pemilik disaring di server dan JS di iframe sandbox | Server tidak pernah menjalankan kode pengguna |
| Data | Uang sebagai bilangan bulat rupiah, waktu UTC, audit log hanya-tambah, pesanan lunas terkunci, `outlet_id` di semua tabel sejak awal | Mencegah migrasi besar saat multi-cabang datang |

**Mode alur**, dua pengaturan yang berdiri sendiri dan ditanyakan di wizard awal:

- Pesanan QR: nyala atau mati. Jika mati, pelanggan langsung ke kasir dan mendapat nomor antrean harian.
- Alur: "Kasir saja" (struk diserahkan ke dapur secara manual) atau "Sampai dapur" (tiket dan layar dapur).

**Pembayaran.** Pencatatan manual tersedia: tunai, QRIS statis atau EDC yang dikonfirmasi kasir. Payment gateway (Midtrans atau Xendit) tersedia sebagai opsi dengan webhook yang idempoten dan terverifikasi tanda tangannya, rekonsiliasi berkala, dan butuh URL HTTPS publik.

**Pengguna per mode.** Mode "Kasir saja" (default) memakai tiga jenis pengguna: owner, kasir, dan pelanggan. Mode "Sampai dapur" menambah pengguna dapur/bar.

**Bahasa.** Antarmuka berbahasa Indonesia dengan terjemahan otomatis Indonesia dan Inggris. Kode dan dokumentasi repositori ditulis dalam bahasa Inggris saat dirilis.

**Printer.** Printer ESC/POS yang populer di Indonesia disambungkan lewat LAN. Sumber daya cetak disediakan sebagai pustaka sehingga perangkat lain bisa disambungkan sesuai pilihan pengguna.

**Pajak.** Tarif, ambang omzet, dan dasar pajak diisi owner sesuai peraturan daerahnya masing-masing.

## Lisensi dan kontribusi

Produk ini open source dengan lisensi AGPL-3.0. Lisensi ini dipilih karena aplikasi dijalankan sebagai layanan web: pihak yang memodifikasi dan menjalankannya untuk orang lain wajib membuka perubahannya.

| Persyaratan | Rincian |
| --- | --- |
| Lisensi | File LICENSE berisi teks AGPL-3.0, dan setiap file kode memakai header SPDX |
| Tautan sumber | Footer aplikasi menampilkan tautan ke repositori kode, sesuai kewajiban AGPL untuk pengguna jaringan |
| Kontribusi | Setiap kontributor memakai DCO (Signed-off-by di tiap commit) |
| Dependensi | Lisensi dependensi Rust diperiksa dengan cargo-deny, dan lisensi yang tidak kompatibel dengan AGPL ditolak |
| Dokumen repositori | README, CONTRIBUTING, SECURITY, dan CODE\_OF\_CONDUCT |
| Kebersihan repositori | Tidak ada kunci, file .env, atau data nyata di dalam repositori |
| Nama dan logo | Diatur terpisah dari lisensi, karena AGPL tidak melindungi merek |

## Asumsi, risiko, dan dependensi

**Asumsi** yang belum teruji dan harus divalidasi lewat wawancara dan uji di kafe nyata:

1. Pemilik kafe kecil mau memakai sistem yang dipasang di perangkat sendiri.
2. Kafe yang baru merintis lebih sering memilih alur "Kasir saja" daripada layar dapur.
3. Pelanggan mau memindai QR di meja. Satu-satunya survei yang ditemukan berasal dari AS dan menunjukkan preferensi rendah, jadi mode tanpa pindai (langsung ke kasir) harus tetap kelas satu.
4. Kafe sudah punya atau mau membeli printer struk ESC/POS dan tablet Android.
5. Pemilik atau penjaga kafe sanggup menjaga backup dan pembaruan dengan panduan yang jelas.

| Risiko | Dampak | Mitigasi |
| --- | --- | --- |
| Server kafe mati atau data hilang | Kehilangan data penjualan | Backup otomatis harian, ekspor, dan panduan pemulihan yang diuji |
| Pesanan QR palsu atau spam | Dapur menerima pesanan fiktif | Konfirmasi kasir, batas laju, token meja yang bisa diganti |
| Salah hitung pajak atau service charge | Kafe salah memungut | Tarif dan basis bisa diatur, uji dengan contoh hitung, pemilik memverifikasi tarif ke otoritas daerahnya |
| Kustomisasi HTML dan JS dieksploitasi | Pencurian data atau eksekusi kode | Penyaringan di server, iframe sandbox, CSP ketat, mode aman, tinjauan keamanan sebelum rilis |
| Perubahan aturan pajak atau QRIS | Fitur pajak atau pembayaran perlu diubah | Semua tarif dan ambang jadi pengaturan, bukan kode |
| Status hukum pembayaran | Perizinan jika produk memegang dana | Produk tidak memegang dana. Gateway dan QRIS langsung ke rekening kafe |
| Printer atau perangkat tidak kompatibel | Struk atau laci kas gagal | Daftar perangkat yang diuji, uji cetak di wizard awal |

**Dependensi:**

- Perangkat keras kafe: printer struk ESC/POS di jaringan lokal, tablet atau PC, router.
- Server: PC atau mini PC di kafe, atau VPS untuk multi-cabang.
- Layanan pihak ketiga: Let's Encrypt, opsional Cloudflare Tunnel, Midtrans atau Xendit untuk payment gateway.
- Informasi yang harus dicek pemilik: tarif dan ambang PBJT di daerah kafe, dari otoritas pajak daerah setempat.
