<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>เพิ่มคำบรรยายแปลสดให้วิดีโอที่กำลังเล่น</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="รุ่นล่าสุด"></a>&nbsp;<a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="ยอดดาวน์โหลดทั้งหมด"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="สถานะ CI บน main"></a>&nbsp;<a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="สัญญาอนุญาต MIT"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>&nbsp;<a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>&nbsp;<a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>ดาวน์โหลด Mimi</strong></a> · <a href="https://mimi.yuxino.cn/th/">เว็บไซต์</a> · <a href="../usage.th.md">คู่มือใช้งาน</a></p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md"><strong>ภาษาไทย</strong></a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

**ไม่ต้องสมัครสมาชิกแอปคำบรรยายเพิ่มอีก** Mimi เป็นทางเลือกฟรีและโอเพนซอร์สแทนแอปคำบรรยายแบบเสียเงิน ใช้ API key ของคุณเองเพื่อต่อบริการรู้จำเสียงและแปลภาษา แล้วดูวิดีโอ ไลฟ์ หรือเรียนออนไลน์พร้อมคำบรรยายแปลสดได้

![ภาพตัวอย่างคำบรรยายสองภาษาของ Mimi พร้อมบทสนทนาสมมติ](../assets/readme-preview.png)

ภาพจำลองหน้าจอพร้อมบทสนทนาตัวอย่าง [ดูคำบรรยายและโหมดเต็มอารมณ์ (หน้าตาเวอร์ชันเก่า)](../demos/README.md)

## สิ่งที่ทำได้

- **อ่านสองภาษาคู่กัน**: ดูวิดีโอ ไลฟ์ หรือเรียนออนไลน์ในภาษาต่างประเทศ โดยเลือกแสดงสองภาษา เฉพาะคำแปล หรือเฉพาะต้นฉบับได้
- **เหลือแค่คำบรรยาย**: ย้ายคำบรรยายและปรับขนาดตัวอักษรกับสีได้ โหมดเต็มอารมณ์เหลือเฉพาะข้อความ และคลิกผ่านไปยังแอปด้านล่างได้
- **เลือกเสียงที่ต้องการ**: บนเดสก์ท็อป เลือกแอปใน macOS / Windows 11 หรือเลือกสตรีมการเล่นเสียงใน Linux ได้
- **เก็บไว้อ่านภายหลัง**: บนเดสก์ท็อป เปิดบันทึกคำบรรยายหรือเสียงเพื่อส่งออกเป็น TXT / Markdown หรือ WAV ทั้งสองตัวเลือกปิดไว้เป็นค่าเริ่มต้น

## เริ่มใช้งาน

บริการคลาวด์ต้องใช้ API key ของคุณเองและอาจมีค่าใช้จ่าย เสียงหรือข้อความที่รู้จำได้จะถูกส่งไปยังบริการที่เลือก

Mac ที่ใช้ชิป Apple และรองรับ macOS 26+ สามารถใช้[บริการ Apple ในเครื่องโดยไม่ต้องมี API key](../provider-setup.md#apple-speech) หลังดาวน์โหลดทรัพยากรภาษาแล้ว

1. **ดาวน์โหลดและติดตั้ง**: เลือกไฟล์จาก[หน้าดาวน์โหลด](https://github.com/yuxino/mimi/releases/latest): DMG สำหรับ macOS 13+ (ชิป Apple / Intel), EXE สำหรับ Windows x64 หรือ AppImage / DEB สำหรับ Linux x86_64 โทรศัพท์ Android 10 ขึ้นไป ดู[คู่มือ Android (ภาษาอังกฤษ)](../../android/README.md#start)
2. **ตั้งค่าบริการ**: ใน การตั้งค่า → โมเดลและบริการ เพิ่ม OpenAI, Gemini, xAI หรือบริการอื่นที่รองรับ ใส่ API key ของคุณเองและข้อมูลที่บริการต้องการ แล้วบันทึก ดูวิธีรับคีย์และข้อมูลที่ต้องกรอกใน[คู่มือตั้งค่า (ภาษาอังกฤษ)](../provider-setup.md#choose)
3. **เลือกเสียงและภาษา**: ในการตั้งค่าเสียงและภาษา เลือกภาษาที่พูดและภาษาแปล เสียงคอมพิวเตอร์จะถูกเลือกไว้ก่อน หากต้องการแปลเสียงของคุณเอง ให้เลือกไมโครโฟน หรือเลือกทั้งสองแหล่งเสียงได้
4. **เริ่มคำบรรยาย**: เปิดคำบรรยายสดในหน้าคำบรรยาย อนุญาตสิทธิ์ที่ระบบขอ แล้วเล่นเนื้อหาที่มีเสียงพูด

## ต้องการความช่วยเหลือ?

[คู่มือการใช้งาน](../usage.th.md) ครอบคลุมการติดตั้ง สิทธิ์ การแสดงคำบรรยาย และการบันทึก หากไม่มีคำบรรยายหรือเชื่อมต่อไม่ได้ ให้ดู[ข้อผิดพลาดในการรู้จำเสียง](../usage.th.md#errors) ก่อน หากยังไม่หาย [แจ้งปัญหา](https://github.com/yuxino/mimi/issues)ได้

<details>
<summary>ผู้ร่วมพัฒนาและชุมชน</summary>

## ผู้ร่วมพัฒนา

หากอยากช่วยพัฒนา เริ่มที่[คู่มือร่วมพัฒนา (อังกฤษ/จีน)](../../.github/CONTRIBUTING.md)

ขอบคุณทุกคนที่เขียนโค้ด รายงานปัญหา ทดลองใช้ Mimi หรือแบ่งปันให้ผู้อื่น (๑•̀ㅂ•́)و✧

ขอบคุณเป็นพิเศษแก่ [@yebuwudong](https://github.com/yebuwudong) สำหรับ [แอป Android](https://github.com/yuxino/mimi/pull/37) และ [@LLLin000](https://github.com/LLLin000) สำหรับ [แอนิเมชันคำบรรยาย](https://github.com/yuxino/mimi/pull/67) และ [การปรับปรุงเสียงบน Windows](https://github.com/yuxino/mimi/pull/89)

ขอบคุณ [@Chtholly000](https://github.com/Chtholly000) ที่ปรับปรุง [คำบรรยายต่อเนื่องของ Gemini และการเปลี่ยนการเชื่อมต่อตามกำหนด](https://github.com/yuxino/Mimi/pull/176)

ขอบคุณ [@EmmetZ](https://github.com/EmmetZ) สำหรับการ[แก้ไขตัวควบคุมคำบรรยายบน Wayland](https://github.com/yuxino/Mimi/pull/242) และเพิ่ม[การจับเสียงจากสตรีมการเล่นแต่ละรายการบน Linux](https://github.com/yuxino/Mimi/pull/275) พร้อม[ไอคอนแอปสำหรับสตรีมการเล่น](https://github.com/yuxino/Mimi/pull/304)

ขอบคุณ [@wangmian0](https://github.com/wangmian0) ที่ [ปิดโหมดคิดของ DeepSeek สำหรับการแปลคำบรรยายสด](https://github.com/yuxino/Mimi/pull/270)

ขอบคุณ [@hubumulity](https://github.com/hubumulity) สำหรับ[การตั้งค่าจำนวนคำบรรยายที่แสดง](https://github.com/yuxino/Mimi/pull/303)

ขอบคุณ [@ryannyui](https://github.com/ryannyui) สำหรับ[การแก้ไขความเข้ากันได้ของกราฟิกใน Linux AppImage และการอัปเกรดชุดเครื่องมือ](https://github.com/yuxino/Mimi/pull/312)

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="40" height="40" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="40" height="40" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="40" height="40" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="40" height="40" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="40" height="40" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="40" height="40" alt="@EmmetZ"></a>
  <a href="https://github.com/wangmian0"><img src="../assets/contributors/wangmian0.svg" width="40" height="40" alt="@wangmian0"></a>
  <a href="https://github.com/hubumulity"><img src="../assets/contributors/hubumulity.svg" width="40" height="40" alt="@hubumulity"></a>
  <a href="https://github.com/ryannyui"><img src="../assets/contributors/ryannyui.svg" width="40" height="40" alt="@ryannyui"></a>
</p>

[ผู้ร่วมพัฒนาทั้งหมด](https://github.com/yuxino/mimi/graphs/contributors)

## ชุมชน

ขอบคุณผู้ใช้จาก [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) และ [Eleduck](https://eleduck.com/) ที่ทดลองใช้ Mimi แบ่งปันความคิดเห็น และช่วยบอกต่อ

</details>

[MIT](../../LICENSE) © 2026 yuxino
