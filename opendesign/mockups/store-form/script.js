const form = document.querySelector("#store-form");
const budget = document.querySelector("#budget");
const budgetValue = document.querySelector("#budgetValue");
const summaryService = document.querySelector("#summaryService");
const summaryDelivery = document.querySelector("#summaryDelivery");
const summaryDate = document.querySelector("#summaryDate");
const summaryProducts = document.querySelector("#summaryProducts");
const summaryTotal = document.querySelector("#summaryTotal");
const randomize = document.querySelector("#randomize");
const toast = document.querySelector("#toast");

const samples = [
  {
    name: "测试顾客",
    phone: "1000000001",
    email: "customer@example.com",
    address: "示例路 100 号",
    service: "企业礼盒定制",
    delivery: "店员电话确认",
    timeSlot: "13:00 - 15:00",
    budget: 900,
    notes: "礼盒需要中性包装，每份附一张空白卡片。",
    products: ["季节果酱礼盒", "手冲咖啡豆"]
  },
  {
    name: "试用联系人",
    phone: "1000000002",
    email: "demo@example.com",
    address: "测试街区 12 栋",
    service: "活动包场申请",
    delivery: "活动当天布置",
    timeSlot: "17:00 - 19:00",
    budget: 1500,
    notes: "预计 16 人，现场需要一张长桌和无咖啡因饮品。",
    products: ["午后茶歇盒", "季节果酱礼盒"]
  },
  {
    name: "样例会员",
    phone: "1000000003",
    email: "member@example.com",
    address: "占位大道 8 号",
    service: "到店自取订单",
    delivery: "到店自取",
    timeSlot: "10:00 - 12:00",
    budget: 350,
    notes: "请预留两份低糖甜品，到店后出示测试编号。",
    products: ["手冲咖啡豆", "午后茶歇盒"]
  }
];

const productImages = {
  手冲咖啡豆: "https://images.unsplash.com/photo-1514432324607-a09d9b4aefdd?auto=format&fit=crop&w=240&q=80",
  季节果酱礼盒: "https://images.unsplash.com/photo-1601493700631-2b16ec4b4716?auto=format&amp;fit=crop&amp;w=240&amp;q=80",
  午后茶歇盒: "https://images.unsplash.com/photo-1495474472287-4d71bcdd2085?auto=format&fit=crop&w=240&q=80"
};

function formatCurrency(value) {
  return `¥${Number(value).toLocaleString("zh-CN")}`;
}

function selectedProducts() {
  return [...form.querySelectorAll('input[name="product"]:checked')].map((input) => ({
    name: input.value,
    price: Number(input.dataset.price)
  }));
}

function updateSummary() {
  const data = new FormData(form);
  const products = selectedProducts();
  const subtotal = products.reduce((sum, product) => sum + product.price, 0);
  summaryService.textContent = data.get("service") || "待选择";
  summaryDelivery.textContent = data.get("delivery") || "待选择";
  summaryDate.textContent = data.get("date") || "待选择";
  budgetValue.textContent = formatCurrency(budget.value);
  summaryTotal.textContent = formatCurrency(subtotal);

  if (!products.length) {
    summaryProducts.innerHTML = '<div class="empty-note">还没有选择商品。</div>';
    return;
  }

  summaryProducts.innerHTML = products
    .map(
      (product) => `
        <div class="mini-row">
          <img src="${productImages[product.name]}" alt="${product.name}" />
          <span>
            <strong>${product.name}</strong>
            <span>单项预估</span>
          </span>
          <b>${formatCurrency(product.price)}</b>
        </div>
      `
    )
    .join("");
}

function setValue(selector, value) {
  const field = form.querySelector(selector);
  if (field) field.value = value;
}

function applySample(sample) {
  setValue("#customerName", sample.name);
  setValue("#phone", sample.phone);
  setValue("#email", sample.email);
  setValue("#address", sample.address);
  setValue("#delivery", sample.delivery);
  setValue("#timeSlot", sample.timeSlot);
  setValue("#budget", sample.budget);
  setValue("#notes", sample.notes);

  form.querySelectorAll('input[name="service"]').forEach((input) => {
    input.checked = input.value === sample.service;
  });

  form.querySelectorAll('input[name="product"]').forEach((input) => {
    input.checked = sample.products.includes(input.value);
  });

  updateSummary();
}

function setDefaultDate() {
  const date = new Date();
  date.setDate(date.getDate() + 2);
  form.querySelector("#date").value = date.toISOString().slice(0, 10);
}

function showToast(message) {
  toast.textContent = message;
  toast.classList.add("show");
  window.setTimeout(() => toast.classList.remove("show"), 3600);
}

randomize.addEventListener("click", () => {
  const sample = samples[Math.floor(Math.random() * samples.length)];
  applySample(sample);
  showToast("已填入一组随机测试内容。");
});

form.addEventListener("input", updateSummary);
form.addEventListener("change", updateSummary);
form.addEventListener("reset", () => {
  window.setTimeout(() => {
    setDefaultDate();
    updateSummary();
  });
});

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const id = `SF-${Math.floor(100000 + Math.random() * 900000)}`;
  showToast(`申请已生成：${id}。`);
});

setDefaultDate();
updateSummary();
