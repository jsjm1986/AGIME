// 客服聊天应用
(function() {
  // DOM 元素
  const chatMessages = document.getElementById('chatMessages');
  const messageInput = document.getElementById('messageInput');
  const sendBtn = document.getElementById('sendBtn');
  const typingIndicator = document.getElementById('typingIndicator');
  const quickBtns = document.querySelectorAll('.quick-btn');

  // 当前时间
  function getCurrentTime() {
    const now = new Date();
    const hours = String(now.getHours()).padStart(2, '0');
    const minutes = String(now.getMinutes()).padStart(2, '0');
    return `${hours}:${minutes}`;
  }

  // HTML 转义
  function escapeHtml(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  }

  // 创建用户消息
  function createUserMessage(text) {
    const row = document.createElement('div');
    row.className = 'message-row user';
    row.innerHTML = `
      <div class="message-avatar">👤</div>
      <div class="message-content">${escapeHtml(text)}</div>
      <div class="message-time">${getCurrentTime()}</div>
    `;
    return row;
  }

  // 创建客服消息
  function createAgentMessage(text) {
    const row = document.createElement('div');
    row.className = 'message-row agent';
    row.innerHTML = `
      <div class="message-avatar">👩‍💼</div>
      <div class="message-content">${escapeHtml(text)}</div>
      <div class="message-time">${getCurrentTime()}</div>
    `;
    return row;
  }

  // 添加消息到聊天区
  function addMessage(element) {
    chatMessages.appendChild(element);
    scrollToBottom();
  }

  // 添加系统消息
  function addSystemMessage(text) {
    const row = document.createElement('div');
    row.className = 'system-message';
    row.innerHTML = `<span>${text}</span>`;
    chatMessages.appendChild(row);
    scrollToBottom();
  }

  // 滚动到底部
  function scrollToBottom() {
    chatMessages.scrollTop = chatMessages.scrollHeight;
  }

  // 显示正在输入
  function showTyping() {
    typingIndicator.classList.add('active');
    scrollToBottom();
  }

  // 隐藏正在输入
  function hideTyping() {
    typingIndicator.classList.remove('active');
  }

  // 生成回复内容
  function generateReply(userMsg) {
    const msg = userMsg.toLowerCase();
    
    if (msg.includes('价格') || msg.includes('多少钱') || msg.includes('费用')) {
      return '我们的产品价格根据具体需求而定，基础套餐从 ¥99/月开始。您可以访问我们的定价页面查看详细信息，或者留下您的联系方式，我们的销售顾问会尽快与您联系。';
    }
    
    if (msg.includes('退款') || msg.includes('退货') || msg.includes('退钱')) {
      return '我们支持7天无理由退款。如果您需要退款，请登录您的账户，在"订单管理"中申请退款，或提供您的订单号，我可以帮您处理。';
    }
    
    if (msg.includes('密码') || msg.includes('登录') || msg.includes('账号')) {
      return '如果您忘记了密码，可以点击登录页面的"忘记密码"链接，通过邮箱或手机号重置密码。如果遇到其他登录问题，请告诉我具体的错误提示。';
    }
    
    if (msg.includes('发票')) {
      return '我们可以提供增值税普通发票和专用发票。请在订单完成后30天内，在账户的"发票管理"中提交开票申请。';
    }
    
    if (msg.includes('谢谢') || msg.includes('感谢')) {
      return '不客气！很高兴能帮到您。如果还有其他问题，随时联系我。祝您使用愉快！😊';
    }
    
    if (msg.includes('你好') || msg.includes('在吗') || msg.includes('hi') || msg.includes('hello')) {
      return '您好！欢迎来到我们的客服中心，我是您的智能客服助手。请问有什么可以帮您？';
    }
    
    return '感谢您的咨询。我已记录您的问题，会尽快为您处理。如果您需要更详细的帮助，可以拨打我们的客服热线：400-XXX-XXXX（工作日 9:00-18:00）。';
  }

  // 处理发送消息
  function handleSend() {
    const text = messageInput.value.trim();
    if (!text) return;

    // 添加用户消息
    addMessage(createUserMessage(text));
    messageInput.value = '';
    messageInput.style.height = 'auto';

    // 模拟客服回复
    showTyping();
    
    const reply = generateReply(text);
    
    setTimeout(() => {
      hideTyping();
      addMessage(createAgentMessage(reply));
    }, 1500 + Math.random() * 1000);
  }

  // 绑定事件
  function bindEvents() {
    // 发送按钮
    sendBtn.addEventListener('click', handleSend);
    
    // 回车发送
    messageInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    });
    
    // 快捷按钮
    quickBtns.forEach(btn => {
      btn.addEventListener('click', () => {
        const msg = btn.dataset.msg;
        messageInput.value = msg;
        messageInput.focus();
      });
    });
    
    // 自动调整输入框高度
    messageInput.addEventListener('input', () => {
      messageInput.style.height = 'auto';
      messageInput.style.height = Math.min(messageInput.scrollHeight, 80) + 'px';
    });
  }

  // 初始化
  function init() {
    bindEvents();
    messageInput.focus();
    
    // 添加欢迎消息
    setTimeout(() => {
      addMessage(createAgentMessage('您好！欢迎来到我们的客服中心，我是您的智能客服助手。请问有什么可以帮您？'));
    }, 500);
  }

  // 启动
  init();
})();
ENDOFFILE
