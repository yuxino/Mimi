<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>คำบรรยายแปลสดจากเสียงคอมพิวเตอร์หรือไมโครโฟน</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="รุ่นล่าสุด"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="ยอดดาวน์โหลดทั้งหมด"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="สถานะ CI บน main"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="สัญญาอนุญาต MIT"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md"><strong>ภาษาไทย</strong></a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi เป็นแอปคำบรรยายที่แปลเสียงจากคอมพิวเตอร์หรือไมโครโฟนแบบสด ดูภาพยนตร์ ติดตามไลฟ์ หรือเรียนออนไลน์ พร้อมคำบรรยายที่ลอยอยู่บนหน้าจอ</p>

![หน้าต่างคำบรรยายสองภาษาของ Mimi บนภาพประกอบต้นฉบับ](../assets/readme-preview.png)

## คุณสมบัติ

- เลือกเสียงระบบ ไมโครโฟน หรือทั้งสองอย่าง ค่าเริ่มต้นคือเสียงระบบ การจับเสียงไมโครโฟนต้องเลือกเองอย่างชัดเจน
- แปลเสียงจากแอปที่เลือกบน macOS หรือ Windows 11
- แสดงข้อความต้นฉบับ คำแปล หรือทั้งสองอย่าง
- ปรับตำแหน่ง ขนาด และสีคำบรรยาย หรือให้การคลิกเมาส์ผ่านหน้าต่างคำบรรยายได้
- บันทึกคำบรรยายหรือเสียงไว้ในเครื่องและส่งออกเป็น TXT / WAV เมื่อจำเป็น การบันทึกคำบรรยายและเสียงปิดไว้ตามค่าเริ่มต้น
- รองรับ macOS 13 ขึ้นไป (Apple silicon และ Intel) และ Windows / Linux x86_64

## เริ่มใช้งาน

สำหรับการตั้งค่าครั้งแรก **เราแนะนำให้เริ่มด้วย Alibaba Cloud หรือ Google Gemini** เราทดสอบ Alibaba Cloud ในการใช้งานจริงมากกว่า จากประสบการณ์ของผมจนถึงตอนนี้ Gemini ให้คำบรรยายที่สม่ำเสมอที่สุด

หากใช้ Google Gemini ควรมีการเชื่อมต่อเครือข่ายที่เสถียร ตรวจสอบโควตาที่ใช้ได้ การเรียกเก็บเงิน และ API key ใน [Google AI Studio](https://aistudio.google.com/)

ลิงก์เปิดใช้บริการ วิธีรับข้อมูลรับรอง และโมเดลที่ Mimi ใช้อยู่ ดูได้ใน **[คู่มือตั้งค่าผู้ให้บริการ (ภาษาอังกฤษ)](../provider-setup.md)**

1. เปิด การตั้งค่า → เสียงและการแปล เพิ่มการตั้งค่า ป้อนข้อมูลรับรองที่ผู้ให้บริการต้องการ แล้วบันทึก
2. เลือกภาษาที่รู้จำและภาษาที่จะแปล
3. เล่นเนื้อหาที่มีเสียง แล้วเปิดคำบรรยายสดในหน้าคำบรรยาย บน macOS 14.2 ขึ้นไป การจับเสียงจากทุกแอปหรือแอปที่เลือกใช้สิทธิ์บันทึกเฉพาะเสียงระบบได้ หากมีสิทธิ์บันทึกหน้าจอเดิมอยู่จะใช้สิทธิ์นั้นต่อ macOS รุ่นก่อนหน้านี้ต้องใช้สิทธิ์บันทึกหน้าจอและเสียงระบบ

บริการเสียงบนคลาวด์ต้องใช้ข้อมูลรับรองของคุณเองและจะได้รับเสียงของคุณ อาจมีค่าบริการ Apple Speech รู้จำเสียงในเครื่องบน Mac ที่รองรับ บริการแปลข้อความระยะไกลจะได้รับข้อความที่รู้จำได้ ส่วน Apple Translation เก็บข้อความไว้บน Mac

[การตั้งค่าและวิธีใช้](../usage.th.md) · [Android (ภาษาอังกฤษ)](../../android/README.md) · [รายงานปัญหา](https://github.com/yuxino/mimi/issues) · [ร่วมพัฒนา (ภาษาจีน/อังกฤษ)](../../.github/CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### การรู้จำเสียงในเครื่องด้วย Apple

**Apple Speech** จะปรากฏเมื่อระบบรองรับ: Apple silicon, macOS 26 ขึ้นไป และมีตัวถอดเสียงของระบบที่พร้อมใช้งาน ไม่ต้องใช้ API key สำหรับเสียง หยุดคำบรรยายแล้วเลือกภาษาใน **ภาษาที่รู้จำ** หากยังไม่มีทรัพยากรภาษานั้น ให้กด **ดาวน์โหลดและใช้งาน** Mimi จะดาวน์โหลดทรัพยากรของ Apple และเลือกภาษาให้เมื่อพร้อม หากดาวน์โหลดภาษาไว้แล้ว ให้กด **ตั้งค่าภาษาที่รู้จำ** แล้วเริ่มคำบรรยาย ไม่ต้องไปที่การตั้งค่าระบบ ไม่มีตัวเลือกตรวจจับภาษาอัตโนมัติ ดู [การตั้งค่า Apple Speech (ภาษาอังกฤษ)](../provider-setup.md#apple-speech)

**Apple Translation** เป็นบริการแปลข้อความในเครื่องที่แยกจากบริการเสียง ใช้ได้บน Mac รุ่น Apple silicon ที่รองรับและใช้ macOS 26 ขึ้นไป เลือกบริการนี้ใน **การแปลข้อความ** บันทึกการตั้งค่า และระบุภาษาต้นทางกับปลายทาง Mimi จะแสดงว่าคู่ภาษาพร้อมใช้หรือไม่ กด **ดาวน์โหลดหรือเปิดใช้ภาษา** เพื่อเปิดหน้าต่างยืนยันการตั้งค่าของ Apple โมเดลแปลแยกจากโมเดลเสียง โมเดลที่มีอยู่จะนำมาใช้ต่อ และ Apple จะดาวน์โหลดโมเดลที่ขาดเมื่อคุณยืนยัน การตรวจสอบและเซสชันคำบรรยายจะไม่ขอดาวน์โหลด ไม่ต้องใช้ API key หรือพร็อกซีสำหรับข้อความ

หากต้องการแสดงเฉพาะข้อความต้นฉบับด้วย Apple Speech ให้เลือก **ไม่แปล (ต้นฉบับเท่านั้น)** คุณยังใช้บริการแปลข้อความระยะไกล เช่น [Index-Translate](#try-index-translate) ได้ โดยบริการนั้นจะได้รับข้อความที่รู้จำได้

<a id="try-index-translate"></a>

### ลองใช้ Index-Translate

[Index-Translate](https://github.com/bilibili/Index-Translate#inference) ของ Bilibili มี API แปลสาธารณะฟรีในขณะนี้ (ข้อมูล ณ วันที่ 5 ตุลาคม 2026) ใช้แปลข้อความจากการรู้จำเสียงของ Alibaba Cloud หรือ Apple Speech ได้

ใน **การตั้งค่า → เสียงและการแปล** เปิดการตั้งค่า **Alibaba Cloud** หรือ **Apple Speech** แล้วเลือก **API ที่เข้ากันได้กับ OpenAI** ใน **การแปลข้อความ** ใช้ค่าจาก [ตัวอย่างทางการ](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41):

| ช่อง | ค่า |
| --- | --- |
| ที่อยู่บริการ | `https://index-translate.bilibili.com/v1` |
| ชื่อโมเดล | `Index-Translate-35B-A3B` |
| API Key | เว้นว่าง API สาธารณะยังไม่ต้องยืนยันตัวตนในขณะนี้ |

บันทึก แล้วทดสอบการเชื่อมต่อข้าง **การแปลข้อความ** หากที่อยู่นี้มีคีย์ที่บันทึกไว้อยู่แล้ว ให้ลบด้วย **ลบคีย์การแปล** แล้วบันทึกอีกครั้ง

Index-Translate ใช้แปลข้อความเท่านั้น หากใช้ Alibaba Cloud ต้องเก็บข้อมูลรับรองการรู้จำเสียงไว้ การรู้จำเสียงอาจยังมีค่าบริการ ส่วน Apple Speech ไม่ต้องใช้คีย์ ความพร้อมใช้งานของ API ฟรีขึ้นอยู่กับผู้ให้บริการ

## คำถามที่พบบ่อย

**macOS ขอสิทธิ์บันทึกซ้ำแม้เปิดไว้แล้ว?** ออกจาก Mimi แล้วลบและเพิ่มเฉพาะรายการของ Mimi ใหม่ใน การตั้งค่าระบบ → ความเป็นส่วนตัวและความปลอดภัย → การบันทึกหน้าจอและเสียงระบบ ใช้ `/Applications/mimi.app` สำหรับรุ่นเผยแพร่ หรือ `/Applications/mimi-dev.app` สำหรับการพัฒนา เปิดสิทธิ์ แล้วเปิดแอปเดิมอีกครั้ง ดู [การกู้คืนสิทธิ์](../usage.th.md#macos-permissions-after-an-update)

## ผู้ร่วมพัฒนา

ขอบคุณทุกคนที่เขียนโค้ด รายงานปัญหา ทดลองใช้ Mimi หรือแบ่งปันให้ผู้อื่น (๑•̀ㅂ•́)و✧

ขอบคุณเป็นพิเศษแก่ [@yebuwudong](https://github.com/yebuwudong) สำหรับ [แอป Android](https://github.com/yuxino/mimi/pull/37) และ [@LLLin000](https://github.com/LLLin000) สำหรับ [แอนิเมชันคำบรรยาย](https://github.com/yuxino/mimi/pull/67) และ [การปรับปรุงเสียงบน Windows](https://github.com/yuxino/mimi/pull/89)

ขอบคุณ [@Chtholly000](https://github.com/Chtholly000) ที่ปรับปรุง [คำบรรยายต่อเนื่องของ Gemini และการเปลี่ยนการเชื่อมต่อตามกำหนด](https://github.com/yuxino/Mimi/pull/176)

ขอบคุณ [@EmmetZ](https://github.com/EmmetZ) ที่แก้ไข [ตัวควบคุมที่ไม่ตามหน้าต่างคำบรรยายบน Wayland](https://github.com/yuxino/Mimi/pull/242)

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="64" height="64" alt="@EmmetZ"></a>
</p>

[ผู้ร่วมพัฒนาทั้งหมด](https://github.com/yuxino/mimi/graphs/contributors)

## ชุมชน

ขอบคุณผู้ใช้จาก [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305) และ [Eleduck](https://eleduck.com/) ที่ทดลองใช้ Mimi แบ่งปันความคิดเห็น และช่วยบอกต่อ

[MIT](../../LICENSE) © 2026 yuxino
