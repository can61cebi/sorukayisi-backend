use crate::config::CONFIG;
use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use log::{error, info};
use std::str::FromStr;

// E-posta gönderme servisi
pub struct EmailService {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from_address: Mailbox,
}

impl EmailService {
    pub fn new() -> Self {
        // SMTP kimlik bilgilerini yapılandırma
        let creds = Credentials::new(
            CONFIG.email_username.clone(),
            CONFIG.email_password.clone(),
        );

        // SMTP taşıyıcı oluşturma
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&CONFIG.email_server)
            .unwrap()
            .credentials(creds)
            .build();

        // Gönderen e-posta adresini ayrıştırma
        let from_address = Mailbox::from_str(&CONFIG.email_from).unwrap_or_else(|_| {
            Mailbox::new(
                Some("Soru Kayısı".into()),
                "noreply@sorukayisi.com".parse().unwrap(),
            )
        });

        EmailService {
            mailer,
            from_address,
        }
    }

    // E-posta doğrulama e-postası gönderme
    pub async fn send_verification_email(
        &self,
        to_email: &str,
        username: &str,
        token: &str,
    ) -> Result<(), anyhow::Error> {
        let verification_link = format!(
            "{}/verify-email?token={}",
            CONFIG.frontend_url, token
        );

        let to_address = Mailbox::from_str(to_email)?;

        let email = Message::builder()
            .from(self.from_address.clone())
            .to(to_address)
            .subject("Soru Kayısı - E-posta Doğrulama")
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(format!(
                                "Merhaba {},\n\nSoru Kayısı hesabınızı doğrulamak için lütfen aşağıdaki bağlantıya tıklayın:\n\n{}\n\nTeşekkürler,\nSoru Kayısı Ekibi",
                                username, verification_link
                            )),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(format!(
                                r#"
                                <html>
                                <body style="font-family: Arial, sans-serif; color: #333; max-width: 600px; margin: 0 auto;">
                                    <div style="background-color: #f9d5a7; padding: 20px; text-align: center; border-radius: 5px 5px 0 0;">
                                        <h1 style="color: #8b4513;">Soru Kayısı</h1>
                                    </div>
                                    <div style="padding: 20px; border: 1px solid #ddd; border-top: none; border-radius: 0 0 5px 5px;">
                                        <p>Merhaba <strong>{}</strong>,</p>
                                        <p>Soru Kayısı hesabınızı doğrulamak için lütfen aşağıdaki düğmeye tıklayın:</p>
                                        <p style="text-align: center; margin: 30px 0;">
                                            <a href="{}" style="background-color: #ff9933; color: white; padding: 10px 20px; text-decoration: none; border-radius: 5px; font-weight: bold;">E-posta Adresimi Doğrula</a>
                                        </p>
                                        <p>Veya bu bağlantıyı tarayıcınızda açın:</p>
                                        <p><a href="{}">{}</a></p>
                                        <p>Teşekkürler,<br>Soru Kayısı Ekibi</p>
                                    </div>
                                </body>
                                </html>
                                "#,
                                username, verification_link, verification_link, verification_link
                            )),
                    ),
            )?;

        // E-postayı gönder - send_async yerine send kullanılması gerekir
        match self.mailer.send(email).await {
            Ok(_) => {
                info!("E-posta doğrulama e-postası gönderildi: {}", to_email);
                Ok(())
            }
            Err(e) => {
                error!("E-posta gönderme hatası: {}", e);
                Err(anyhow::anyhow!("E-posta gönderme hatası: {}", e))
            }
        }
    }

    // Öğretmen onay bildirimi gönderme
    pub async fn send_teacher_approval_email(
        &self,
        to_email: &str,
        username: &str,
        is_approved: bool,
    ) -> Result<(), anyhow::Error> {
        let to_address = Mailbox::from_str(to_email)?;

        let (subject, content) = if is_approved {
            (
                "Soru Kayısı - Öğretmen Hesabınız Onaylandı",
                format!(
                    r#"
                    <html>
                    <body style="font-family: Arial, sans-serif; color: #333; max-width: 600px; margin: 0 auto;">
                        <div style="background-color: #f9d5a7; padding: 20px; text-align: center; border-radius: 5px 5px 0 0;">
                            <h1 style="color: #8b4513;">Soru Kayısı</h1>
                        </div>
                        <div style="padding: 20px; border: 1px solid #ddd; border-top: none; border-radius: 0 0 5px 5px;">
                            <p>Merhaba <strong>{}</strong>,</p>
                            <p>Öğretmen hesabınız onaylanmıştır. Artık Soru Kayısı'nda soru setleri oluşturabilir ve oyun başlatabilirsiniz.</p>
                            <p style="text-align: center; margin: 30px 0;">
                                <a href="{}/login" style="background-color: #ff9933; color: white; padding: 10px 20px; text-decoration: none; border-radius: 5px; font-weight: bold;">Giriş Yap</a>
                            </p>
                            <p>Teşekkürler,<br>Soru Kayısı Ekibi</p>
                        </div>
                    </body>
                    </html>
                    "#,
                    username, CONFIG.frontend_url
                )
            )
        } else {
            (
                "Soru Kayısı - Öğretmen Hesabı Talebi",
                format!(
                    r#"
                    <html>
                    <body style="font-family: Arial, sans-serif; color: #333; max-width: 600px; margin: 0 auto;">
                        <div style="background-color: #f9d5a7; padding: 20px; text-align: center; border-radius: 5px 5px 0 0;">
                            <h1 style="color: #8b4513;">Soru Kayısı</h1>
                        </div>
                        <div style="padding: 20px; border: 1px solid #ddd; border-top: none; border-radius: 0 0 5px 5px;">
                            <p>Merhaba <strong>{}</strong>,</p>
                            <p>Öğretmen hesabı talebiniz reddedilmiştir. Bunun bir hata olduğunu düşünüyorsanız, lütfen bizimle iletişime geçin.</p>
                            <p>Öğrenci olarak giriş yapmak için:</p>
                            <p style="text-align: center; margin: 30px 0;">
                                <a href="{}/login" style="background-color: #ff9933; color: white; padding: 10px 20px; text-decoration: none; border-radius: 5px; font-weight: bold;">Giriş Yap</a>
                            </p>
                            <p>Teşekkürler,<br>Soru Kayısı Ekibi</p>
                        </div>
                    </body>
                    </html>
                    "#,
                    username, CONFIG.frontend_url
                )
            )
        };

        let email = Message::builder()
            .from(self.from_address.clone())
            .to(to_address)
            .subject(subject)
            .header(ContentType::TEXT_HTML)
            .body(content)?;

        // E-postayı gönder - send_async yerine send kullanılması gerekir
        match self.mailer.send(email).await {
            Ok(_) => {
                info!("Öğretmen onay e-postası gönderildi: {}", to_email);
                Ok(())
            }
            Err(e) => {
                error!("E-posta gönderme hatası: {}", e);
                Err(anyhow::anyhow!("E-posta gönderme hatası: {}", e))
            }
        }
    }

    // Şifre sıfırlama e-postası gönderme
    pub async fn send_password_reset_email(
        &self,
        to_email: &str,
        username: &str,
        token: &str,
    ) -> Result<(), anyhow::Error> {
        let reset_link = format!(
            "{}/reset-password?token={}",
            CONFIG.frontend_url, token
        );

        let to_address = Mailbox::from_str(to_email)?;

        let email = Message::builder()
            .from(self.from_address.clone())
            .to(to_address)
            .subject("Soru Kayısı - Şifre Sıfırlama")
            .header(ContentType::TEXT_HTML)
            .body(format!(
                r#"
                <html>
                <body style="font-family: Arial, sans-serif; color: #333; max-width: 600px; margin: 0 auto;">
                    <div style="background-color: #f9d5a7; padding: 20px; text-align: center; border-radius: 5px 5px 0 0;">
                        <h1 style="color: #8b4513;">Soru Kayısı</h1>
                    </div>
                    <div style="padding: 20px; border: 1px solid #ddd; border-top: none; border-radius: 0 0 5px 5px;">
                        <p>Merhaba <strong>{}</strong>,</p>
                        <p>Şifrenizi sıfırlamak için aşağıdaki bağlantıya tıklayın:</p>
                        <p style="text-align: center; margin: 30px 0;">
                            <a href="{}" style="background-color: #ff9933; color: white; padding: 10px 20px; text-decoration: none; border-radius: 5px; font-weight: bold;">Şifremi Sıfırla</a>
                        </p>
                        <p>Bu bağlantı 24 saat boyunca geçerlidir.</p>
                        <p>Şifre sıfırlama talebinde bulunmadıysanız, lütfen bu e-postayı dikkate almayın.</p>
                        <p>Teşekkürler,<br>Soru Kayısı Ekibi</p>
                    </div>
                </body>
                </html>
                "#,
                username, reset_link
            ))?;

        // E-postayı gönder - send_async yerine send kullanılması gerekir
        match self.mailer.send(email).await {
            Ok(_) => {
                info!("Şifre sıfırlama e-postası gönderildi: {}", to_email);
                Ok(())
            }
            Err(e) => {
                error!("E-posta gönderme hatası: {}", e);
                Err(anyhow::anyhow!("E-posta gönderme hatası: {}", e))
            }
        }
    }

    // Oyun davet e-postası gönderme (öğretmenler için)
    pub async fn send_game_invitation(
        &self,
        to_email: &str,
        username: &str,
        game_code: &str,
        game_title: &str,
    ) -> Result<(), anyhow::Error> {
        let game_link = format!("{}/game/join?code={}", CONFIG.frontend_url, game_code);

        let to_address = Mailbox::from_str(to_email)?;

        let email = Message::builder()
            .from(self.from_address.clone())
            .to(to_address)
            .subject(format!("Soru Kayısı - Oyun Davetiyesi: {}", game_title))
            .header(ContentType::TEXT_HTML)
            .body(format!(
                r#"
                <html>
                <body style="font-family: Arial, sans-serif; color: #333; max-width: 600px; margin: 0 auto;">
                    <div style="background-color: #f9d5a7; padding: 20px; text-align: center; border-radius: 5px 5px 0 0;">
                        <h1 style="color: #8b4513;">Soru Kayısı</h1>
                    </div>
                    <div style="padding: 20px; border: 1px solid #ddd; border-top: none; border-radius: 0 0 5px 5px;">
                        <p>Merhaba <strong>{}</strong>,</p>
                        <p>Bir oyuna davet edildiniz: <strong>{}</strong></p>
                        <p>Oyun kodu: <strong>{}</strong></p>
                        <p style="text-align: center; margin: 30px 0;">
                            <a href="{}" style="background-color: #ff9933; color: white; padding: 10px 20px; text-decoration: none; border-radius: 5px; font-weight: bold;">Oyuna Katıl</a>
                        </p>
                        <p>Öğrencileriniz de bu kodu kullanarak oyuna katılabilirler.</p>
                        <p>Teşekkürler,<br>Soru Kayısı Ekibi</p>
                    </div>
                </body>
                </html>
                "#,
                username, game_title, game_code, game_link
            ))?;

        // E-postayı gönder - send_async yerine send kullanılması gerekir
        match self.mailer.send(email).await {
            Ok(_) => {
                info!("Oyun davet e-postası gönderildi: {}", to_email);
                Ok(())
            }
            Err(e) => {
                error!("E-posta gönderme hatası: {}", e);
                Err(anyhow::anyhow!("E-posta gönderme hatası: {}", e))
            }
        }
    }
}