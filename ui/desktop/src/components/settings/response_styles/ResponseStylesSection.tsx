import { useEffect, useState } from 'react';
import { useResponseStyles, ResponseStyleSelectionItem } from './ResponseStyleSelectionItem';

export const ResponseStylesSection = () => {
  const responseStyles = useResponseStyles();
  const [currentStyle, setCurrentStyle] = useState('concise');

  useEffect(() => {
    const savedStyle = localStorage.getItem('response_style');
    if (savedStyle) {
      try {
        setCurrentStyle(savedStyle);
      } catch (error) {
        console.error('Error parsing response style:', error);
      }
    } else {
      // Set default to concise for new users
      localStorage.setItem('response_style', 'concise');
      setCurrentStyle('concise');
    }
  }, []);

  const handleStyleChange = async (newStyle: string) => {
    setCurrentStyle(newStyle);
    localStorage.setItem('response_style', newStyle);

    // Dispatch custom event to notify other components of the change
    window.dispatchEvent(new CustomEvent('responseStyleChanged'));
  };

  return (
    <div className="space-y-2">
      {responseStyles.map((style) => (
        <ResponseStyleSelectionItem
          key={style.key}
          style={style}
          currentStyle={currentStyle}
          showDescription={true}
          handleStyleChange={handleStyleChange}
        />
      ))}
    </div>
  );
};
