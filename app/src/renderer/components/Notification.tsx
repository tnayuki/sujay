import React from 'react';
import './Notification.css';

interface NotificationProps {
  message: string;
}

const Notification: React.FC<NotificationProps> = ({ message }) => {
  return (
    <div className="notification">
      <div className="notification-content">
        ℹ️  {message}
      </div>
    </div>
  );
};

export default Notification;
